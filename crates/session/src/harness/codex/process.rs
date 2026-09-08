//! Own Codex and every descendant, drain stderr concurrently with JSONL stdout.
use crate::{process::OwnedSupervisor, SessionState};
use anyhow::{bail, Context, Result};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    task::JoinHandle,
};

pub struct Running {
    pub child: Child,
    pub lines: Lines<BufReader<ChildStdout>>,
    stderr: JoinHandle<std::io::Result<String>>,
    stdin: JoinHandle<std::io::Result<()>>,
    owner: Option<OwnedSupervisor>,
    pid: u32,
}

pub fn binary_path(
    envs: &std::collections::BTreeMap<String, String>,
    workdir: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let workdir = std::env::current_dir()?.join(workdir);
    let path = envs
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate =
            workdir
                .join(directory)
                .join(if cfg!(windows) { "codex.exe" } else { "codex" });
        if let Ok(meta) = candidate.metadata() {
            if !meta.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Ok(candidate);
        }
    }
    bail!("Codex executable unavailable on execution node PATH")
}

pub fn spawn(
    session: &SessionState,
    prompt: String,
    images: &[std::path::PathBuf],
) -> Result<Running> {
    let binary = binary_path(&session.harness.envs, &session.working_dir)?;
    let (mut cmd, lease) = match crate::process::command(&binary)? {
        Some((cmd, lease)) => (cmd, Some(lease)),
        None => (Command::new(&binary), None),
    };
    cmd.arg("exec");
    if let Some(id) = &session.harness.fork_from {
        cmd.args(["fork", id]);
    } else if let Some(id) = &session.harness.thread_id {
        cmd.args(["resume", id]);
    }
    cmd.args(["--json", "--skip-git-repo-check"]);
    if session.harness.thread_id.is_none() && session.harness.fork_from.is_none() {
        cmd.args(["--color", "never"]);
    }
    if let Some(model) = &session.harness.model {
        cmd.args(["--model", model]);
    }
    for image in images {
        cmd.arg("--image").arg(image);
    }
    cmd.arg("-")
        .current_dir(&session.working_dir)
        .envs(
            &session
                .env_passthrough
                .iter()
                .cloned()
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .envs(&session.harness.envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !session.tools_path.is_empty() {
        let inherited = session
            .harness
            .envs
            .get("PATH")
            .cloned()
            .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let mut paths = session.tools_path.clone();
        paths.extend(std::env::split_paths(&inherited));
        cmd.env("PATH", std::env::join_paths(paths)?);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    crate::process::configure_owned_command(&mut cmd, lease.is_some());
    let mut child = cmd
        .spawn()
        .context("cannot start Codex binary on execution node")?;
    let pid = child.id().context("Codex process missing PID")?;
    let owner = lease.map(|l| l.spawned(child.id())).transpose()?;
    let mut input = child.stdin.take().context("Codex stdin missing")?;
    let stdin = tokio::spawn(async move {
        input.write_all(prompt.as_bytes()).await?;
        input.shutdown().await
    });
    let mut error = child.stderr.take().context("Codex stderr missing")?;
    let stderr = tokio::spawn(async move {
        let mut tail = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = error.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buf[..n]);
            if tail.len() > 65536 {
                tail.drain(..tail.len() - 65536);
            }
        }
        Ok(String::from_utf8_lossy(&tail).into_owned())
    });
    let lines = BufReader::new(child.stdout.take().context("Codex stdout missing")?).lines();
    Ok(Running {
        child,
        lines,
        stderr,
        stdin,
        owner,
        pid,
    })
}
impl Running {
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(owner) = &mut self.owner {
            owner.terminate();
        } else {
            self.kill_group();
        }
        tokio::time::timeout(std::time::Duration::from_secs(30), self.child.wait())
            .await
            .context("Codex process cleanup timed out")??;
        Ok(())
    }
    pub async fn finish(&mut self) -> Result<(std::process::ExitStatus, String)> {
        let status = self.child.wait().await?;
        let stderr = (&mut self.stderr)
            .await
            .context("Codex stderr task failed")??;
        let input = (&mut self.stdin).await.context("Codex stdin task failed")?;
        if status.success() {
            input?;
        }
        Ok((status, stderr))
    }
    fn kill_group(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(owner) = &mut self.owner {
            owner.terminate();
        } else {
            self.kill_group();
        }
        self.stdin.abort();
        self.stderr.abort();
    }
}
