//! Killable embedded VM transport. The installed agent supplies the worker
//! entrypoint, so Python requires neither a host interpreter nor another binary.
use super::{error_result, run_vm_step, StepCtx, StepResult};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize)]
struct Request {
    run_id: String,
    step_name: String,
    workflow_root: PathBuf,
    context: serde_json::Value,
    code: String,
}

// The worker response is JSON transport, not the user's `output.json`.
// JSON escaping can expand each allowed UTF-8 stream byte to six ASCII bytes.
const WORKER_RESPONSE_LIMIT_BYTES: usize = crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES
    * 12
    + crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES * 2
    + 1024 * 1024;

#[cfg(test)]
const TEST_WORKER_ENV: &str = "OPENCODER_PYTHON_TEST_WORKER";
#[cfg(test)]
const TEST_RESPONSE_MARKER: &[u8] = b"\x1eOPENCODER_PYTHON_RESPONSE\x1e";

/// Called before node initialization or logging; stdout is the result channel.
pub fn worker_main() -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
        "output_limit_exceeded: python request JSON exceeds {} bytes",
        crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
    );
    let request: Request = serde_json::from_slice(&bytes).context("invalid Python step request")?;
    let result = run_vm_step(
        &request.run_id,
        &request.step_name,
        &request.workflow_root,
        &request.context,
        &request.code,
    );
    let encoded = serialize_json_bounded(&result, WORKER_RESPONSE_LIMIT_BYTES, "python response")?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&encoded)?;
    stdout.flush()?;
    Ok(())
}

#[cfg(not(test))]
fn worker_binary() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let filename = format!("opencoder-agent{}", std::env::consts::EXE_SUFFIX);
    if executable
        .file_name()
        .is_some_and(|name| name == filename.as_str())
    {
        return Ok(executable);
    }
    // Library integration tests live under target/<profile>/deps. Other
    // embedders must install the agent alongside their own executable.
    let directory = executable.parent().context("executable has no directory")?;
    let directory = if directory.file_name().is_some_and(|name| name == "deps") {
        directory
            .parent()
            .context("test executable has no profile directory")?
    } else {
        directory
    };
    let candidate = directory.join(filename);
    anyhow::ensure!(
        candidate.is_file(),
        "isolated Python worker missing at {}; build/install opencoder-agent first",
        candidate.display()
    );
    Ok(candidate)
}

fn worker_command() -> Result<(Command, Option<opencoder_session::process::SpawnLease>)> {
    #[cfg(test)]
    let (program, args): (PathBuf, Vec<OsString>) = {
        // Unit tests execute a hidden entrypoint in this exact libtest binary.
        // The worker therefore always uses the current source/build, including
        // in a clean target directory with no prebuilt opencoder-agent.
        (
            std::env::current_exe()?,
            vec![
                "--exact".into(),
                "exec::python::process::tests::isolated_python_test_worker_entrypoint".into(),
                "--nocapture".into(),
            ],
        )
    };
    #[cfg(not(test))]
    let (program, args): (PathBuf, Vec<OsString>) =
        (worker_binary()?, vec!["internal-python-step".into()]);
    let (mut command, lease) = match opencoder_session::process::command(&program)? {
        Some((command, lease)) => (command, Some(lease)),
        None => (Command::new(program), None),
    };
    command.args(args);
    #[cfg(test)]
    command.env(TEST_WORKER_ENV, "1");
    Ok((command, lease))
}

fn decode_worker_response(bytes: &[u8]) -> Result<StepResult> {
    #[cfg(test)]
    let bytes = {
        let offset = bytes
            .windows(TEST_RESPONSE_MARKER.len())
            .position(|window| window == TEST_RESPONSE_MARKER)
            .context("test worker response marker missing")?;
        &bytes[offset + TEST_RESPONSE_MARKER.len()..]
    };
    serde_json::from_slice(bytes).context("decode embedded VM result")
}

pub(super) async fn execute(ctx: &StepCtx, code: String, cancel: CancellationToken) -> StepResult {
    match execute_inner(ctx, code, cancel).await {
        Ok(result) => result,
        Err(error) => error_result(format!("python worker: {error:#}")),
    }
}

async fn execute_inner(
    ctx: &StepCtx,
    code: String,
    cancel: CancellationToken,
) -> Result<StepResult> {
    if cancel.is_cancelled() {
        return Ok(cancelled());
    }
    if ctx.step.timeout_secs == Some(0) {
        return Ok(error_result("python step timeout".into()));
    }
    let request = serialize_json_bounded(
        &Request {
            run_id: ctx.run_id.clone(),
            step_name: ctx.step.name.clone(),
            workflow_root: ctx.workflow_root.clone(),
            context: ctx.context(),
            code,
        },
        crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
        "python request JSON",
    )?;
    let (mut command, lease) = worker_command()?;
    let supervised = lease.is_some();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    opencoder_session::process::configure_owned_command(&mut command, supervised);
    #[cfg(unix)]
    if lease.is_none() {
        command.process_group(0);
    }
    #[cfg(target_os = "linux")]
    if lease.is_none() {
        unsafe {
            let parent = libc::getpid();
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(std::io::Error::other("node exited before VM startup"));
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().context("spawn embedded VM")?;
    let mut owner = match lease {
        Some(lease) => ProcessOwner::Supervised(lease.spawned(child.id())?),
        None => ProcessOwner::Direct(child.id()),
    };
    let mut input = child.stdin.take().context("worker stdin")?;
    let output = child.stdout.take().context("worker stdout")?;
    let errors = child.stderr.take().context("worker stderr")?;
    let deadline = async {
        match ctx.step.timeout_secs {
            Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        _ = deadline => Some(Err(anyhow::anyhow!("python step timeout"))),
        result = async {
            let write = async move {
                input.write_all(&request).await?;
                input.shutdown().await?;
                Ok::<(), anyhow::Error>(())
            };
            let wait = async { Ok::<_, anyhow::Error>(child.wait().await?) };
            let (_, stdout, stderr, status) = tokio::try_join!(
                write,
                crate::sandbox::output_limit::read_bounded(
                    output,
                    "python worker response",
                    WORKER_RESPONSE_LIMIT_BYTES,
                ),
                crate::sandbox::output_limit::read_bounded(
                    errors,
                    "python worker stderr",
                    crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
                ),
                wait,
            )?;
            anyhow::ensure!(status.success(), "VM exited {status}: {}", String::from_utf8_lossy(&stderr));
            decode_worker_response(&stdout)
        } => Some(result),
    };
    // Kill descendants and reap the direct child on every path, including
    // protocol/pipe errors. Never release the node slot with a VM still running.
    owner.terminate();
    if child.try_wait()?.is_none() {
        opencoder_session::process::wait_owned_child(&mut child, owner.is_supervised())
            .await
            .context("reap embedded VM owner")?;
    }
    result.unwrap_or_else(|| Ok(cancelled()))
}

fn serialize_json_bounded<T: Serialize>(
    value: &T,
    limit: usize,
    label: &'static str,
) -> Result<Vec<u8>> {
    let mut writer = BoundedWriter::new(limit, label);
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.into_inner())
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    label: &'static str,
}

impl BoundedWriter {
    fn new(limit: usize, label: &'static str) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            limit,
            label,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(std::io::Error::other(format!(
                "output_limit_exceeded: {} exceeds {} bytes",
                self.label, self.limit
            )));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn cancelled() -> StepResult {
    StepResult {
        outcome: opencoder_dag::StepOutcome::Cancelled,
        ..error_result("python step cancelled".into())
    }
}

enum ProcessOwner {
    Direct(Option<u32>),
    Supervised(opencoder_session::process::OwnedSupervisor),
}

impl ProcessOwner {
    fn is_supervised(&self) -> bool {
        matches!(self, Self::Supervised(_))
    }

    fn terminate(&mut self) {
        match self {
            Self::Direct(pid) =>
            {
                #[cfg(unix)]
                if let Some(pid) = pid.take() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
            }
            Self::Supervised(supervisor) => supervisor.terminate(),
        }
    }
}

impl Drop for ProcessOwner {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_python_test_worker_entrypoint() {
        if std::env::var_os(TEST_WORKER_ENV).is_none() {
            return;
        }
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(TEST_RESPONSE_MARKER).unwrap();
        stdout.flush().unwrap();
        match worker_main() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
    }

    #[test]
    fn structured_json_generation_limit_is_inclusive() {
        let exact = "x".repeat(30);
        let encoded = serialize_json_bounded(&exact, 32, "test JSON").unwrap();
        assert_eq!(encoded.len(), 32);

        let over = "x".repeat(31);
        let error = serialize_json_bounded(&over, 32, "test JSON").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output_limit_exceeded: test JSON exceeds 32 bytes"),
            "{error:#}"
        );
    }
}
