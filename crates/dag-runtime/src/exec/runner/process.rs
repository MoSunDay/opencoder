//! Foreground process ownership, with optional systemd lifecycle containment.
use anyhow::{Context, Result};
use opencoder_core::harness::RunnerSettings;
use opencoder_session::process::OwnedSupervisor;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

pub struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    pub lines: Lines<BufReader<ChildStdout>>,
    stderr: JoinHandle<std::io::Result<String>>,
    owner: Option<OwnedSupervisor>,
    unit: Option<String>,
}

pub fn unit(settings: &RunnerSettings) -> Option<String> {
    settings
        .parent_unit
        .as_ref()
        .map(|_| format!("opencoder-runner-{}.service", ulid::Ulid::new()))
}

impl Running {
    pub async fn spawn(
        settings: &RunnerSettings,
        unit: Option<String>,
        timeout: u64,
        input: &serde_json::Value,
    ) -> Result<Self> {
        let mut command = settings.command.clone();
        if let Some(unit) = &unit {
            let parent = settings.parent_unit.as_ref().unwrap();
            command = [
                vec![
                    "systemd-run".into(),
                    "--wait".into(),
                    "--pipe".into(),
                    "--collect".into(),
                    "--quiet".into(),
                    "--unit".into(),
                    unit.clone(),
                    "--property".into(),
                    format!("BindsTo={parent}"),
                    "--property".into(),
                    format!("After={parent}"),
                    "--property".into(),
                    "KillMode=control-group".into(),
                    "--property".into(),
                    "TimeoutStopSec=10".into(),
                    "--property".into(),
                    format!("RuntimeMaxSec={}", timeout.saturating_add(30)),
                    "--property".into(),
                    format!("WorkingDirectory={}", settings.workdir.display()),
                    "--property".into(),
                    "UMask=0077".into(),
                ],
                settings
                    .envs
                    .iter()
                    .flat_map(|(k, v)| ["--setenv".into(), format!("{k}={v}")])
                    .collect(),
                vec!["--".into()],
                command,
            ]
            .concat();
        }
        let (mut cmd, lease) = match opencoder_session::process::command(&command[0])? {
            Some((cmd, lease)) => (cmd, Some(lease)),
            None => (Command::new(&command[0]), None),
        };
        cmd.args(&command[1..])
            .current_dir(&settings.workdir)
            .envs(&settings.envs)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        cmd.process_group(0);
        opencoder_session::process::configure_owned_command(&mut cmd, lease.is_some());
        let mut child = cmd.spawn().context("start registered Runner")?;
        let owner = lease.map(|lease| lease.spawned(child.id())).transpose()?;
        let stdin = child.stdin.take().context("Runner stdin unavailable")?;
        let stdout = child.stdout.take().context("Runner stdout unavailable")?;
        let mut errors = child.stderr.take().context("Runner stderr unavailable")?;
        let stderr = tokio::spawn(async move {
            let mut tail = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = errors.read(&mut buffer).await?;
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..n]);
                if tail.len() > 65536 {
                    tail.drain(..tail.len() - 65536);
                }
            }
            Ok(String::from_utf8_lossy(&tail).into_owned())
        });
        let mut running = Self {
            child,
            stdin: Some(stdin),
            lines: BufReader::new(stdout).lines(),
            stderr,
            owner,
            unit,
        };
        let mut bytes = serde_json::to_vec(input)?;
        bytes.push(b'\n');
        running.stdin.as_mut().unwrap().write_all(&bytes).await?;
        Ok(running)
    }

    pub async fn finish(&mut self) -> Result<String> {
        self.stdin.take();
        let status = tokio::time::timeout(Duration::from_secs(30), self.child.wait())
            .await
            .context("Runner exit timed out")??;
        let errors = (&mut self.stderr).await??;
        anyhow::ensure!(status.success(), "Runner exited {status}: {errors}");
        self.unit = None;
        Ok(errors)
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut input) = self.stdin.take() {
            let _ = input.write_all(b"{\"type\":\"cancel\"}\n").await;
        }
        if let Ok(status) = tokio::time::timeout(Duration::from_secs(10), self.child.wait()).await {
            status.context("wait for cancelled Runner")?;
            self.cleanup_unit().await?;
            return Ok(());
        }
        self.cleanup_unit().await?;
        if let Some(owner) = &mut self.owner {
            owner.terminate();
        } else if let Some(pid) = self.child.id() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
            self.child.start_kill()?;
        }
        tokio::time::timeout(Duration::from_secs(30), self.child.wait())
            .await
            .context("Runner cleanup timed out")??;
        Ok(())
    }

    async fn cleanup_unit(&mut self) -> Result<()> {
        if let Some(unit) = &self.unit {
            let output = tokio::time::timeout(
                Duration::from_secs(30),
                Command::new("systemctl").args(["stop", unit]).output(),
            )
            .await
            .context("Runner service cleanup timed out")??;
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::ensure!(
                output.status.success()
                    || error.contains("not loaded")
                    || error.contains("not found"),
                "Runner service cleanup failed: {error}"
            );
            self.unit = None;
        }
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(unit) = &self.unit {
            let _ = std::process::Command::new("systemctl")
                .args(["stop", "--no-block", unit])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        if let Some(owner) = &mut self.owner {
            owner.terminate();
        } else if let Some(pid) = self.child.id() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
        self.stderr.abort();
    }
}
