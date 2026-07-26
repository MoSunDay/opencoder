use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::io::AsyncReadExt;

use super::bg::{handoff, BgState};

pub struct BashTool;

/// Merge captured stdout and stderr into one string, prefixing stderr with a
/// `[stderr]` marker so the two streams stay distinguishable. Empty inputs
/// produce empty output (no placeholder) so callers can decide their own
/// "no output" framing.
fn merge_streams(stdout: &str, stderr: &str) -> String {
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("[stderr]\n");
        combined.push_str(stderr);
    }
    combined
}

/// Maximum per-command timeout (seconds) for the bash tool.
///
/// Must stay strictly below `DEFAULT_TOOL_TIMEOUT` (600 s) in
/// [`crate::runner`] — the runner's outer `biased select!` polls the deadline
/// arm before the exec arm, so if bash's own `tokio::time::timeout` were
/// ≥ 600 s the safety net would fire first, drop the exec future, and
/// `kill_on_drop(true)` would silently kill the child — bypassing handoff
/// entirely (the "moved to background" path never runs). Capping at 590 s
/// guarantees the handoff code path always wins the race.
pub(crate) const BASH_MAX_TIMEOUT_SECS: u64 = 590;

/// Compile-time guard: the bash cap must be strictly below the runner's outer
/// safety-net timeout. If someone lowers `DEFAULT_TOOL_TIMEOUT` below 590,
/// this assertion fails at compile time.
const _: () = assert!(
    BASH_MAX_TIMEOUT_SECS < crate::runner::DEFAULT_TOOL_TIMEOUT.as_secs(),
    "BASH_MAX_TIMEOUT_SECS must be strictly below DEFAULT_TOOL_TIMEOUT"
);

/// Resolve the per-command timeout from tool input, applying the default
/// (120 s) and the hard cap ([`BASH_MAX_TIMEOUT_SECS`]).
fn resolve_timeout_secs(input: &Value) -> u64 {
    input
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .min(BASH_MAX_TIMEOUT_SECS)
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Executes a bash command in the session working directory and returns stdout+stderr. Use for git, builds, tests, running scripts. Commands run non-interactively."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "command".into(),
            json::prop_str("The bash command to execute."),
        );
        props.insert(
            "workdir".into(),
            json::prop_str("Optional working directory override."),
        );
        props.insert("timeout".into(), serde_json::json!({ "type": "number", "description": "Maximum runtime in seconds before the command is auto-backgrounded. Default 120, hard-capped at 590. Exceeding the cap does NOT kill the command — it keeps running in the background with output captured to a file." }));
        json::object_schema(Value::Object(props), &["command"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.trim().is_empty() {
            return Ok(ToolOutput::err("empty command"));
        }
        let workdir = input
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let timeout_secs = resolve_timeout_secs(&input);

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-lc")
            .arg(command)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Detach the child from the controlling terminal. stdout/stderr are
        // already piped above, but without setsid() the child still shares our
        // controlling terminal and can write straight to /dev/tty (sudo prompts,
        // progress bars, login-shell greetings, backgrounded children). Those
        // bytes bypass our pipes and land on the alt screen at the cursor
        // position — i.e. inside the TUI composer/input area. Running the child
        // in its own session makes /dev/tty unavailable, forcing all output
        // through the pipes we capture.
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        // Spawn explicitly (instead of `cmd.output()`) so we control the timeout
        // kill: `kill_on_drop` only signals the *direct* bash child, leaving
        // grandchildren (builds, servers, backgrounded jobs) as orphans. Because
        // `setsid()` above made the child a session + process-group leader, its
        // process-group id equals its pid, so `kill(-pgid, SIGKILL)` reaps the
        // whole descendant tree on timeout.
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        #[cfg(unix)]
        let pgid = pid as libc::pid_t;

        // Shared capture state: incremental drain tasks push 8 KiB chunks here.
        // In the foreground phase this only buffers; after `handoff` the file
        // handle is set and pushes also write to the output file.
        let state = Arc::new(Mutex::new(BgState::new()));

        // Drain the pipes concurrently with `wait()`. Without concurrent reads a
        // process that emits more than the pipe buffer (~64 KiB) would deadlock:
        // it blocks on write, `wait()` never returns, and we hang until timeout.
        // Incremental reads (instead of read_to_end) let us hand off a still-
        // running command to the background supervisor without losing the pipe.
        let stdout_task: tokio::task::JoinHandle<()> = {
            let state = Arc::clone(&state);
            let mut pipe = child.stdout.take().expect("stdout was piped");
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match pipe.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut st = state.lock().unwrap();
                            st.push_stdout(&buf[..n]);
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        let stderr_task: tokio::task::JoinHandle<()> = {
            let state = Arc::clone(&state);
            let mut pipe = child.stderr.take().expect("stderr was piped");
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match pipe.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut st = state.lock().unwrap();
                            st.push_stderr(&buf[..n]);
                        }
                        Err(_) => break,
                    }
                }
            })
        };

        let exit_status =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
                Ok(r) => r?,
                Err(_) => {
                    // Timed out: instead of killing the group, hand the child
                    // (and its drain tasks + capture state) off to a detached
                    // background supervisor. The supervisor keeps the command
                    // running, streams output to /tmp/opencode_bg_<pid>.output,
                    // and cleans up the process group when the command exits.
                    #[cfg(unix)]
                    {
                        handoff(
                            pid,
                            pgid,
                            ctx.session_id.clone(),
                            child,
                            stdout_task,
                            stderr_task,
                            state,
                        );
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = (pid, child, stdout_task, stderr_task, state);
                    }
                    let path = super::bg::output_path(pid);
                    let msg = format!(
                        "Command exceeded {timeout_secs}s — moved to background. \
                         PID: {pid}. Check progress: cat {}. \
                         Cleaned up automatically when it exits.",
                        path.display()
                    );
                    return Ok(ToolOutput::ok(msg));
                }
            };

        // Normal completion: await drain tasks (they resolve at EOF when the
        // child exits) then read the captured buffers.
        let _ = stdout_task.await;
        let _ = stderr_task.await;
        let (stdout, stderr) = {
            let st = state.lock().unwrap();
            (
                String::from_utf8_lossy(&st.stdout_buf).to_string(),
                String::from_utf8_lossy(&st.stderr_buf).to_string(),
            )
        };
        let code = exit_status.code().unwrap_or(-1);
        let streams = merge_streams(&stdout, &stderr);
        let combined = if streams.is_empty() {
            format!("(no output)\n[exit code: {code}]")
        } else {
            format!("{streams}\n[exit code: {code}]")
        };
        let is_error = code != 0;
        Ok(opencoder_core::tool::truncate_output_with_error(
            combined,
            ctx.max_output,
            is_error,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::ToolContext;
    use serde_json::json;

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            message_id: "test".into(),
            agent: "act".into(),
            working_dir: std::env::current_dir().unwrap(),
            max_output: 100_000,
            proxy: None,
        }
    }

    #[tokio::test]
    async fn bash_normal_completion() {
        let tool = BashTool;
        let input = json!({"command": "echo hello; echo world >&2"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(!out.is_error, "expected success, got: {}", out.content);
        assert!(out.content.contains("hello"), "stdout: {}", out.content);
        assert!(
            out.content.contains("[stderr]"),
            "stderr marker: {}",
            out.content
        );
        assert!(
            out.content.contains("world"),
            "stderr text: {}",
            out.content
        );
        assert!(
            out.content.contains("[exit code: 0]"),
            "exit code: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_handoff_on_timeout() {
        let tool = BashTool;
        // Use a 1-second timeout on a 10-second sleep to trigger handoff fast.
        let input = json!({"command": "sleep 3; echo done", "timeout": 1});
        let out = tool.execute(input, &ctx()).await.unwrap();
        // Handoff is not an error — the model gets a guidance message.
        assert!(
            !out.is_error,
            "handoff should be ToolOutput::ok, got: {}",
            out.content
        );
        assert!(
            out.content.contains("moved to background"),
            "missing handoff text: {}",
            out.content
        );
        // Extract pid from the message and check the output path is mentioned.
        assert!(
            out.content.contains("/tmp/opencode_bg_"),
            "missing output path: {}",
            out.content
        );
        // The background supervisor writes [exit code: N] when the child exits.
        // Wait for the output file to contain it (the sleep is 3s, so poll).
        let pid_str = out
            .content
            .split("PID: ")
            .nth(1)
            .and_then(|s| s.split('.').next())
            .unwrap_or("");
        let pid: u32 = pid_str.parse().unwrap_or(0);
        assert!(pid > 0, "could not parse pid from: {}", out.content);
        let path = super::super::bg::output_path(pid);
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        let mut got_exit_code = false;
        while std::time::Instant::now() < deadline {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains("[exit code:") {
                    got_exit_code = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(
            got_exit_code,
            "output file never received [exit code:]: {:?}",
            std::fs::read_to_string(&path).ok()
        );
        // Clean up the temp file.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn timeout_clamped_below_safety_net() {
        // Default when absent.
        assert_eq!(resolve_timeout_secs(&json!({})), 120);
        // Sub-cap values pass through unchanged.
        assert_eq!(resolve_timeout_secs(&json!({"timeout": 60})), 60);
        assert_eq!(resolve_timeout_secs(&json!({"timeout": 300})), 300);
        assert_eq!(resolve_timeout_secs(&json!({"timeout": 590})), 590);
        // Values at or above the runner safety net (600 s) must be clamped
        // so the bash handoff always fires before the biased outer deadline.
        assert_eq!(
            resolve_timeout_secs(&json!({"timeout": 600})),
            BASH_MAX_TIMEOUT_SECS
        );
        assert_eq!(
            resolve_timeout_secs(&json!({"timeout": 9999})),
            BASH_MAX_TIMEOUT_SECS
        );
        assert_eq!(
            resolve_timeout_secs(&json!({"timeout": u64::MAX})),
            BASH_MAX_TIMEOUT_SECS
        );
    }
}
