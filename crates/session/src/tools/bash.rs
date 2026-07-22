use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::io::AsyncReadExt;

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

/// After a timeout we've already `kill(-pgid)`'d the whole group, so the pipe
/// write-ends close and the drain tasks resolve with EOF. Await them (bounded,
/// in case a grandchild that escaped the group kill still holds a write-end) to
/// recover whatever output the command produced before timing out — far more
/// useful for diagnosing a hanging build/test than a bare "timed out" message.
async fn drain_partial(task: tokio::task::JoinHandle<Vec<u8>>) -> String {
    match tokio::time::timeout(Duration::from_millis(500), task).await {
        Ok(Ok(v)) => String::from_utf8_lossy(&v).to_string(),
        // Join error (task panicked) or bounded-timeout expiry: nothing safely
        // recoverable — report empty rather than risk wedging the tool.
        _ => String::new(),
    }
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
        props.insert("timeout".into(), serde_json::json!({ "type": "number", "description": "Optional timeout in seconds (default 120)." }));
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
        let timeout_secs = input.get("timeout").and_then(|v| v.as_u64()).unwrap_or(120);

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
        #[cfg(unix)]
        let pgid = child.id().unwrap_or(0) as libc::pid_t;

        // Drain the pipes concurrently with `wait()`. Without concurrent reads a
        // process that emits more than the pipe buffer (~64 KiB) would deadlock:
        // it blocks on write, `wait()` never returns, and we hang until timeout.
        let stdout_task: tokio::task::JoinHandle<Vec<u8>> = {
            let mut pipe = child.stdout.take().expect("stdout was piped");
            tokio::spawn(async move {
                let mut v = Vec::new();
                let _ = pipe.read_to_end(&mut v).await;
                v
            })
        };
        let stderr_task: tokio::task::JoinHandle<Vec<u8>> = {
            let mut pipe = child.stderr.take().expect("stderr was piped");
            tokio::spawn(async move {
                let mut v = Vec::new();
                let _ = pipe.read_to_end(&mut v).await;
                v
            })
        };

        let exit_status =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
                Ok(r) => r?,
                Err(_) => {
                    // Timed out: signal the entire process group. A negative pid
                    // means "send to every process in the group". `kill_on_drop` is
                    // kept above as a last-resort net for the direct child should
                    // this path unwind.
                    #[cfg(unix)]
                    unsafe {
                        let _ = libc::kill(-pgid, libc::SIGKILL);
                    }
                    // Reap the direct child so it does not become a zombie; the rest
                    // of the group is reparented to init and reaped there.
                    let _ = child.wait().await;
                    // Recover partial output: after the group kill the pipe
                    // write-ends close and the drain tasks resolve. Whatever the
                    // command printed before timing out is usually the key clue to
                    // *why* it hung, so surface it instead of discarding it.
                    let stdout = drain_partial(stdout_task).await;
                    let stderr = drain_partial(stderr_task).await;
                    let partial = merge_streams(&stdout, &stderr);
                    let msg = if partial.is_empty() {
                        format!("command timed out after {timeout_secs}s")
                    } else {
                        format!("command timed out after {timeout_secs}s\n{partial}")
                    };
                    return Ok(opencoder_core::tool::truncate_output_with_error(
                        msg,
                        ctx.max_output,
                        true,
                    ));
                }
            };

        let stdout = String::from_utf8_lossy(&stdout_task.await.expect("stdout drain")).to_string();
        let stderr = String::from_utf8_lossy(&stderr_task.await.expect("stderr drain")).to_string();
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
