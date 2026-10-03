//! Local non-interactive command execution for the TUI.
//!
//! Pure functions: `run_command` runs the host shell synchronously (with a
//! timeout), `spawn` offloads it to a background task so the UI stays
//! responsive. No state is held here.

use std::path::Path;
use std::time::Duration;

use tokio::sync::oneshot;

const TIMEOUT_SECS: u64 = 10;

/// Run `cmd` via the host shell in `workdir`, merging stdout and stderr.
///
/// Returns the combined output string. Special cases:
/// - empty output → `(no output)`
/// - timeout → `timeout (Ns)`
/// - spawn error → `error: <message>`
pub async fn run_command(cmd: &str, workdir: &Path) -> String {
    let output = opencoder_session::tools::command::host::run(
        cmd,
        Some(workdir),
        Duration::from_secs(TIMEOUT_SECS),
    )
    .await;

    match output {
        Ok(out) => {
            let mut text = String::new();
            use std::fmt::Write;
            let _ = write!(&mut text, "{}", String::from_utf8_lossy(&out.stdout));
            let _ = write!(&mut text, "{}", String::from_utf8_lossy(&out.stderr));
            let text = text.trim_end().to_string();
            if text.is_empty() {
                "(no output)".to_string()
            } else {
                text
            }
        }
        Err(e) if e.to_string().contains("timed out") => format!("timeout ({TIMEOUT_SECS}s)"),
        Err(e) => format!("error: {e}"),
    }
}

/// Spawn `cmd` in the background and return a receiver for its output.
///
/// The returned [`oneshot::Receiver`] resolves to the same string that
/// [`run_command`] produces. Poll it with `try_recv` in the event loop.
pub fn spawn(cmd: &str, workdir: &Path) -> oneshot::Receiver<String> {
    let (tx, rx) = oneshot::channel();
    let cmd = cmd.to_string();
    let workdir = workdir.to_path_buf();
    tokio::spawn(async move {
        let out = run_command(&cmd, &workdir).await;
        let _ = tx.send(out);
    });
    rx
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_command_captures_stdout() {
        let d = tempfile::tempdir().unwrap();
        let out = run_command("echo hello", d.path()).await;
        assert_eq!(out, "hello");
    }

    #[tokio::test]
    async fn run_command_merges_stderr() {
        let d = tempfile::tempdir().unwrap();
        let out = run_command("echo out; echo err 1>&2", d.path()).await;
        assert!(out.contains("out"));
        assert!(out.contains("err"));
    }

    #[tokio::test]
    async fn run_command_empty_output() {
        let d = tempfile::tempdir().unwrap();
        let out = run_command("true", d.path()).await;
        assert_eq!(out, "(no output)");
    }

    #[tokio::test]
    async fn run_command_times_out() {
        let d = tempfile::tempdir().unwrap();
        let out = run_command("sleep 30", d.path()).await;
        assert!(out.starts_with("timeout"));
    }

    #[tokio::test]
    async fn spawn_returns_output() {
        let d = tempfile::tempdir().unwrap();
        let rx = spawn("echo spawned", d.path());
        let out = rx.await.unwrap();
        assert_eq!(out, "spawned");
    }
}
