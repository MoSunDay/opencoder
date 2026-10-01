use super::Receipt;
use crate::step_log::{StepOutputLog, Stream};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub struct StepProcess {
    pub key: String,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: String,
    pub timeout_secs: Option<u64>,
}

pub async fn execute(
    root: &Path,
    process: StepProcess,
    cancel: CancellationToken,
    output: Option<StepOutputLog>,
) -> Result<(i32, String)> {
    let StepProcess {
        key,
        argv,
        env,
        cwd,
        timeout_secs,
    } = process;
    let deadline = timeout_secs
        .map(|seconds| {
            tokio::time::Instant::now()
                .checked_add(Duration::from_secs(seconds))
                .context("DAG step timeout is out of range")
        })
        .transpose()?;
    ensure!(!cancel.is_cancelled(), "DAG step cancelled");
    ensure!(
        !key.is_empty()
            && key.len() <= 128
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid step execution key"
    );
    let receipt: Receipt = serde_json::from_slice(&std::fs::read(root.join("container.json"))?)?;
    let private = root.join("private/exec");
    let process_file = private.join(format!("{key}.json"));
    let pid_file = private.join(format!("{key}.pid"));
    let _cleanup = Files(vec![process_file.clone(), pid_file.clone()]);
    let mut args = vec!["/usr/bin/dag-runner".to_string(), "step".to_string()];
    args.extend(argv);
    let mut environment = vec![
        "PATH=/usr/local/bin:/usr/bin:/bin".to_string(),
        "HOME=/tmp".to_string(),
    ];
    environment.extend(env.into_iter().map(|(key, value)| format!("{key}={value}")));
    let process = json!({"terminal":false,"user":{"uid":receipt.agent_uid,"gid":65532},
        "args":args,"env":environment,"cwd":cwd,"noNewPrivileges":true,
        "capabilities":super::config::capabilities()});
    opencoder_core::atomic_write(&process_file, &serde_json::to_vec(&process)?)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&process_file, std::fs::Permissions::from_mode(0o600))?;
    let mut child = Command::new("runc")
        .arg("--root")
        .arg(root.join("runc-state"))
        .args(["exec", "--process"])
        .arg(&process_file)
        .arg("--pid-file")
        .arg(&pid_file)
        .arg(&receipt.id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("execute step in DAG container")?;
    let stdout = child.stdout.take().context("DAG stdout missing")?;
    let stderr = child.stderr.take().context("DAG stderr missing")?;
    let stdout: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match &output {
        Some(log) => Box::new(log.tee_reader(Stream::Stdout, stdout)),
        None => Box::new(stdout),
    };
    let stderr: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match &output {
        Some(log) => Box::new(log.tee_reader(Stream::Stderr, stderr)),
        None => Box::new(stderr),
    };
    let captured_stdout = super::output::Capture::default();
    let captured_stderr = super::output::Capture::default();
    let stdout = captured_stdout.reader(stdout);
    let stderr = captured_stderr.reader(stderr);
    let deadline = async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(anyhow::anyhow!("DAG step cancelled")),
        _ = deadline => Err(anyhow::anyhow!("DAG step timeout")),
        result = async {
            let (out, err, status) = tokio::try_join!(
                crate::sandbox::output_limit::read_logged(stdout,"DAG stdout",crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,None),
                crate::sandbox::output_limit::read_logged(stderr,"DAG stderr",crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,None),
                async { Ok::<_,anyhow::Error>(child.wait().await?) },
            )?;
            let mut text = String::from_utf8_lossy(&out).into_owned();
            if !err.is_empty() { text.push_str(&String::from_utf8_lossy(&err)); }
            Ok((status.code().unwrap_or(-1), text))
        } => result,
    };
    if result.is_err() {
        let mut pid = None;
        for _attempt in 0..50 {
            if let Ok(value) = std::fs::read_to_string(&pid_file) {
                pid = value.trim().parse::<i32>().ok().filter(|pid| *pid > 1);
                break;
            }
            if child.try_wait()?.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(pid) = pid {
            signal(pid, libc::SIGTERM)?;
        }
        match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                super::super::runc::delete_force(&root.join("runc-state"), &receipt.id).await?;
                child.kill().await?;
                anyhow::bail!("step process tree cleanup failed; DAG container stopped");
            }
        }
    }
    result.map_err(|error| {
        super::ProcessFailure::new(error, captured_stdout.text() + &captured_stderr.text()).into()
    })
}

fn signal(pid: i32, signal: i32) -> Result<()> {
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error).context("open DAG step pidfd");
    }
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            descriptor,
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    unsafe {
        libc::close(descriptor);
    }
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("signal DAG step");
        }
    }
    Ok(())
}

struct Files(Vec<std::path::PathBuf>);
impl Drop for Files {
    fn drop(&mut self) {
        for file in &self.0 {
            if let Err(error) = std::fs::remove_file(file) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::error!(%error, "remove private step launch file");
                }
            }
        }
    }
}
