use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::io::AsyncReadExt;

#[cfg(test)]
use super::bg::{kill_all, list, test_registry_mutex};
use super::bg::{handoff, output_path, register, unregister, BgState};

/// Foreground timeout for a bash command (seconds). When exceeded the command
/// is moved to the background (see [`super::bg::handoff`]) instead of being
/// killed — long-running builds keep going and their output lands in a temp
/// file the model can read later. The runner keeps bash exempt from its own
/// 600 s leaf-tool safety net (`runner::execute`) so the two deadlines never
/// race; bash has its own shorter internal timeout.
///
/// Overridden to 1 s in unit tests so the handoff path is exercisable without
/// a 130 s wait. Integration tests (`tests/`) link the non-`cfg(test)` build
/// and see the real 130 s value.
#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 130;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 1;

/// Marker prefix the runner looks for to deduplicate consecutive bash-timeout
/// tool results (see `runner::dedup_consecutive_bash_timeouts`). The closing
/// `]` is part of the full message, not the marker itself.
pub(crate) const BASH_TIMEOUT_MARKER: &str = "[bash-timeout:";

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
            json::prop_str("Optional working directory override. Defaults to the session working directory, so only pass this to run a command in a different directory; no need for a manual `cd`."),
        );
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
        // On non-unix there is no process group; `pid` only feeds the registry
        // below, so sink it to avoid an unused-variable warning.
        #[cfg(not(unix))]
        let _ = pid;

        // Register the live process so the display-only `/ps` can list it and
        // `/stop` can kill its process group while it runs in the foreground.
        // The entry is removed below once `wait()` returns (or by `/stop`).
        #[cfg(unix)]
        register(pid, pgid, ctx.session_id.clone());

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

        // Race the natural exit against a foreground timeout. When the command
        // exceeds BASH_TIMEOUT_SECS it is moved to the background (not killed)
        // so long-running builds keep going; the model is told where to find
        // the output. The runner does not impose its 600 s leaf-tool deadline
        // on bash either (see `runner::execute`) — bash has its own shorter
        // internal timeout, avoiding a race between the two deadlines.
        let exit_status = match tokio::time::timeout(
            Duration::from_secs(BASH_TIMEOUT_SECS),
            child.wait(),
        )
        .await
        {
            Ok(r) => r?,
            Err(_) => {
                // Timeout — hand the still-running command to the background
                // supervisor so it keeps running. Capture whatever output has
                // accumulated so far to include in the message.
                #[cfg(unix)]
                {
                    let captured = {
                        let st = state.lock().unwrap();
                        let stdout = String::from_utf8_lossy(&st.stdout_buf);
                        let stderr = String::from_utf8_lossy(&st.stderr_buf);
                        merge_streams(&stdout, &stderr)
                    };
                    handoff(
                        pid,
                        pgid,
                        ctx.session_id.clone(),
                        child,
                        stdout_task,
                        stderr_task,
                        state,
                    );
                    return Ok(ToolOutput {
                        content: format!(
                            "{BASH_TIMEOUT_MARKER} command timed out after {BASH_TIMEOUT_SECS}s \u{2014} moved to background]\n\
                             pid: {pid}\noutput: {}\n\n{captured}",
                            output_path(pid).display()
                        ),
                        // Not an error — the command is still running in the
                        // background. This keeps the tool-failure guard from
                        // tripping on legitimate long-running builds.
                        is_error: false,
                        images: vec![],
                    });
                }
                #[cfg(not(unix))]
                {
                    let _ = child.kill().await;
                    return Ok(ToolOutput {
                        content: format!(
                            "{BASH_TIMEOUT_MARKER} command timed out after {BASH_TIMEOUT_SECS}s \u{2014} killed]",
                        ),
                        is_error: false,
                        images: vec![],
                    });
                }
            }
        };
        // Natural completion (or a `/stop` group-kill, which makes `wait()`
        // return a signal exit): remove the registry entry. Idempotent — a
        // `/stop` that already removed it is a harmless no-op.
        #[cfg(unix)]
        unregister(pid);

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
        // Success (code == 0): no exit-code annotation — success is implicit.
        // Failure (code != 0): append `[exit code: N]` so the model sees the
        // failure and can react to it.
        let combined = if code == 0 {
            if streams.is_empty() {
                "(no output)".to_string()
            } else {
                streams
            }
        } else if streams.is_empty() {
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
            !out.content.contains("[exit code:"),
            "success must not annotate exit code: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_failure_appends_exit_code() {
        let tool = BashTool;
        let input = json!({"command": "echo oops; exit 7"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(
            out.is_error,
            "expected error for non-zero exit: {}",
            out.content
        );
        assert!(out.content.contains("oops"), "stdout: {}", out.content);
        assert!(
            out.content.contains("[exit code: 7]"),
            "failure must annotate exit code: {}",
            out.content
        );
    }

    /// A short command (completes well under the test timeout of 1 s) returns
    /// its own output without triggering the timeout/handoff path.
    #[tokio::test]
    async fn bash_short_command_completes_normally() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        let input = json!({"command": "echo done"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(!out.is_error, "expected success, got: {}", out.content);
        assert!(
            out.content.contains("done"),
            "expected the command's own output, got: {}",
            out.content
        );
        assert!(
            !out.content.contains(BASH_TIMEOUT_MARKER),
            "short command must not trigger timeout handoff: {}",
            out.content
        );
    }

    /// A command that exceeds the foreground timeout is handed off to the
    /// background supervisor: the output contains the timeout marker, the pid,
    /// and the background output file path. The command keeps running (not
    /// killed) and is registered for `/ps` / `/stop`.
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_timeout_triggers_handoff() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        // sleep 3 exceeds the 1 s test timeout (BASH_TIMEOUT_SECS == 1 under
        // cfg(test)).
        let input = json!({"command": "sleep 3"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(
            !out.is_error,
            "timeout is not an error — command still runs in background: {}",
            out.content
        );
        assert!(
            out.content.contains(BASH_TIMEOUT_MARKER),
            "timeout output must contain the marker: {}",
            out.content
        );
        assert!(
            out.content.contains("pid:"),
            "timeout output must contain the pid: {}",
            out.content
        );
        assert!(
            out.content.contains("output:"),
            "timeout output must contain the output path label: {}",
            out.content
        );
        assert!(
            out.content.contains("/tmp/opencoder_bg_"),
            "timeout output must reference the background output file: {}",
            out.content
        );
        // Clean up: kill the backgrounded process so it does not linger.
        kill_all();
    }

    /// While a command is running it is registered in the background registry
    /// (so `/ps` lists it / `/stop` can kill it); once it exits the entry is
    /// removed. Verified by writing the child pid (`$$` == the setsid leader
    /// the tool spawned) to a file and inspecting the registry mid-flight.
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_registers_while_running_unregisters_after() {
        let _g = test_registry_mutex().lock().await;
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let tool = BashTool;
        let mut c = ctx();
        c.working_dir = dir.path().to_path_buf();
        // sleep 0.5 finishes well within the 1 s test timeout
        // (BASH_TIMEOUT_SECS == 1 under cfg(test)) so the command completes
        // in the foreground — no handoff, no timeout marker.
        let input = json!({
            "command": format!("echo $$ > {pf}; sleep 0.5; echo done", pf = pidfile.display())
        });
        // Run the tool concurrently so we can inspect the registry mid-flight.
        let handle = tokio::spawn(async move { tool.execute(input, &c).await.unwrap() });

        // Wait for the command to start and write its pid.
        let mut pid: u32 = 0;
        for _ in 0..60 {
            if let Ok(txt) = std::fs::read_to_string(&pidfile) {
                if let Ok(p) = txt.trim().parse::<u32>() {
                    pid = p;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(pid > 0, "pidfile never written: command did not start");

        // While running, the live pid must be registered for `/ps` / `/stop`.
        assert!(
            list().iter().any(|i| i.pid == pid),
            "running bash pid {pid} should be registered"
        );

        let out = handle.await.unwrap();
        assert!(out.content.contains("done"), "{}", out.content);

        // After completion the registry entry is removed.
        assert!(
            !list().iter().any(|i| i.pid == pid),
            "completed bash should have unregistered pid {pid}"
        );
    }

    #[test]
    fn parameters_schema_hides_timeout_from_model() {
        // `timeout` is intentionally not a model-facing property: bash uses a
        // fixed internal timeout (BASH_TIMEOUT_SECS) that hands long-running
        // commands to the background. Exposing `timeout` would let the model
        // raise it arbitrarily (a known past failure mode); `command`/`workdir`
        // stay exposed. This guard prevents re-introduction.
        let schema = BashTool.parameters();
        let props = schema
            .get("properties")
            .expect("schema has a properties object");
        assert!(
            props.get("command").is_some(),
            "command must remain in schema"
        );
        assert!(
            props.get("workdir").is_some(),
            "workdir must remain in schema"
        );
        assert!(
            props.get("timeout").is_none(),
            "timeout must NOT be exposed in the model-facing schema"
        );
    }
}
