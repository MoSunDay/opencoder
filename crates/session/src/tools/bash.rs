use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

use super::bg::{drain_output, handoff, output_path, BgState, OutputStream};
#[cfg(test)]
use super::bg::{kill_all, list, test_registry_mutex};

pub(super) mod process_group;
mod timeout;

#[cfg(unix)]
use process_group::ProcessGroupGuard;

/// Default enforced foreground deadline for a bash command (seconds). Command
/// text may override it through [`timeout::resolve`]. When the
/// resolved deadline is exceeded the command is moved to the background
/// (see [`super::bg::handoff`]) instead of being killed — long-running
/// builds keep going and their output lands in a temp file the model can read
/// later. The runner keeps bash exempt from its own 600 s leaf-tool safety net
/// (`runner::execute`) so the two deadlines never race; bash has its own
/// shorter internal timeout.
///
/// NOTE: when no command hint is present, the number *shown to the model* is
/// [`BASH_TIMEOUT_DISPLAY_SECS`] (120 s), deliberately *lower* than this real
/// deadline (130 s). The ~10 s gap is a buffer so the model is nudged to treat
/// a command as "timed out → backgrounded" a little before the hard cutoff
/// actually fires. Do NOT "fix" the two numbers to be equal — the mismatch is
/// intentional and asserted at compile time below.
///
/// Overridden to 1 s in unit tests so the handoff path is exercisable without
/// a 130 s wait. Integration tests (`tests/`) link the non-`cfg(test)` build
/// and see the real 130 s value.
#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 130;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 1;

/// Default model-visible timeout (seconds), used when command text supplies no
/// dynamic hint. Deliberately *lower* than the default enforced deadline
/// ([`BASH_TIMEOUT_SECS`]); see that constant's doc comment for the buffer
/// rationale. Same 1 s override under `cfg(test)`.
#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_DISPLAY_SECS: u64 = 120;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_DISPLAY_SECS: u64 = 1;

/// Compile-time guarantee the display value stays strictly below the real
/// deadline — the ~10 s buffer is load-bearing. Only checked outside
/// `cfg(test)` (where both collapse to 1 s and the relation is meaningless);
/// a plain `cargo build` (not `cargo test`) enforces it.
#[cfg(not(test))]
const _: () = assert!(BASH_TIMEOUT_DISPLAY_SECS < BASH_TIMEOUT_SECS);

/// Marker prefix the runner looks for to deduplicate consecutive bash-timeout
/// tool results (see `runner::dedup_consecutive_bash_timeouts`). The closing
/// `]` is part of the full message, not the marker itself.
pub(crate) const BASH_TIMEOUT_MARKER: &str = "[bash-timeout:";

pub struct BashTool;

/// Prepend the agent-tools PATH export to the script bash actually runs.
/// The tool runs `bash -lc` — a LOGIN shell recomputes `PATH` from
/// `/etc/profile`, so env injection (`cmd.env("PATH", …)`) would be dropped
/// before the user command executes; the only durable channel is a literal
/// `export PATH=…` line at the top of the script itself.
///
/// Fail-safe sanitization: `"` or `$` in the joined dirs skips injection
/// entirely (they would break out of the double-quoted assignment). The
/// dirs come from our own agents-root pools, but a hostile path must never
/// become script text. `None`/empty ⇒ the command verbatim.
fn script_with_tools_path(command: &str, tools_path: Option<&str>) -> String {
    match tools_path {
        Some(joined) if !joined.is_empty() && !joined.contains('"') && !joined.contains('$') => {
            format!("export PATH=\"{joined}\":$PATH\n{command}")
        }
        _ => command.to_string(),
    }
}

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
        // Infer a per-command foreground deadline from raw `timeout N` or
        // `sleep N` text. The pure scanner is deliberately small (no regex or
        // shell parser), and never rewrites the command passed to bash.
        let resolved = timeout::resolve(command, BASH_TIMEOUT_SECS, BASH_TIMEOUT_DISPLAY_SECS);
        if resolved.command.trim().is_empty() {
            return Ok(ToolOutput::err("empty command"));
        }
        let workdir = input
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        // The EXECUTED script gets the agent-tools PATH export prepended; the
        // timeout scanner above and every display/echo surface keep the
        // ORIGINAL command text (`ToolStart` carries the raw input JSON).
        let script = script_with_tools_path(resolved.command, ctx.tools_path.as_deref());

        let supervised = crate::process::command("bash")?;
        let (mut cmd, lease) = match supervised {
            Some((command, lease)) => (command, Some(lease)),
            None => (tokio::process::Command::new("bash"), None),
        };
        let supervised = lease.is_some();
        cmd.arg("-lc")
            .arg(script)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Step-scoped contract vars from workflow-orchestrated
            // sessions (e.g. OPENCODER_HOW_APPEND); login-shell profiles
            // don't scrub unknown OPENCODER_* names, so they survive `-l`.
            .envs(ctx.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        crate::process::configure_owned_command(&mut cmd, supervised);

        // Detach the child from the controlling terminal. stdout/stderr are
        // already piped above, but without setsid() the child still shares our
        // controlling terminal and can write straight to /dev/tty (sudo prompts,
        // progress bars, login-shell greetings, backgrounded children). Those
        // bytes bypass our pipes and land on the alt screen at the cursor
        // position — i.e. inside the TUI composer/input area. Running the child
        // in its own session makes /dev/tty unavailable, forcing all output
        // through the pipes we capture.
        #[cfg(unix)]
        if lease.is_none() {
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        // Spawn explicitly (instead of `cmd.output()`) so we control the timeout
        // kill: `kill_on_drop` only signals the *direct* bash child, leaving
        // grandchildren (builds, servers, backgrounded jobs) as orphans. Because
        // `setsid()` above made the child a session + process-group leader, its
        // process-group id equals its pid, so `kill(-pgid, SIGKILL)` reaps the
        // whole descendant tree on timeout.
        let mut child = cmd.spawn()?;
        // Guard against a None pid: `unwrap_or(0)` would make `kill(-0, ...)`
        // target our own process group. A successful `spawn()` always yields a
        // pid on Unix, so this early return is purely defensive.
        let pid = match child.id() {
            Some(p) => p,
            None => return Ok(ToolOutput::err("failed to get child pid")),
        };
        #[cfg(unix)]
        let pgid = pid as libc::pid_t;
        // On non-unix there is no process group; `pid` only feeds the registry
        // below, so sink it to avoid an unused-variable warning.
        #[cfg(not(unix))]
        let _ = pid;

        // RAII owns both descendant termination and registry cleanup. Moving
        // this guard into handoff transfers ownership to the supervisor;
        // dropping this future on cancellation performs the same cleanup.
        #[cfg(unix)]
        let mut process_group = if let Some(lease) = lease {
            ProcessGroupGuard::registered_supervised(
                pid,
                ctx.session_id.clone(),
                lease.spawned(child.id())?,
            )?
        } else {
            ProcessGroupGuard::registered(pid, pgid, ctx.session_id.clone())
        };

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
            let pipe = child.stdout.take().expect("stdout was piped");
            tokio::spawn(drain_output(pipe, state, OutputStream::Stdout, pgid))
        };
        let stderr_task: tokio::task::JoinHandle<()> = {
            let state = Arc::clone(&state);
            let pipe = child.stderr.take().expect("stderr was piped");
            tokio::spawn(drain_output(pipe, state, OutputStream::Stderr, pgid))
        };

        // Race the natural exit against the resolved foreground timeout. When
        // the command exceeds it, it is moved to the background (not killed)
        // so long-running builds keep going; the model is told where to find
        // the output. The runner does not impose its 600 s leaf-tool deadline
        // on bash either (see `runner::execute`) — bash has its own shorter
        // internal timeout, avoiding a race between the two deadlines.
        let output_limit = state.lock().unwrap().output_limit_token();
        enum ForegroundResult {
            Exited(std::io::Result<std::process::ExitStatus>),
            TimedOut,
            OutputLimit,
        }
        let foreground = tokio::select! {
            biased;
            _ = output_limit.cancelled() => ForegroundResult::OutputLimit,
            result = child.wait() => ForegroundResult::Exited(result),
            _ = tokio::time::sleep(Duration::from_secs(resolved.timeout_secs)) => {
                ForegroundResult::TimedOut
            }
        };
        let exit_status = match foreground {
            ForegroundResult::Exited(result) => result?,
            ForegroundResult::OutputLimit => {
                #[cfg(unix)]
                process_group.terminate();
                let _ = child.wait().await;
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                })
                .await;
                let error = state
                    .lock()
                    .unwrap()
                    .output_limit_error()
                    .unwrap_or("output_limit_exceeded")
                    .to_string();
                return Ok(ToolOutput::err(error));
            }
            ForegroundResult::TimedOut => {
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
                    if let Err(error) =
                        handoff(pid, child, stdout_task, stderr_task, state, process_group).await
                    {
                        return Ok(ToolOutput::err(error));
                    }
                    return Ok(ToolOutput {
                        content: format!(
                            "{BASH_TIMEOUT_MARKER} command timed out after {}s \u{2014} moved to background]\n\
                             pid: {pid}\noutput: {}\n\n{captured}",
                            resolved.display_secs,
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
                            "{BASH_TIMEOUT_MARKER} command timed out after {}s \u{2014} killed]",
                            resolved.display_secs,
                        ),
                        is_error: false,
                        images: vec![],
                    });
                }
            }
        };
        // Natural completion (or `/stop`) releases the entire process group.
        // This also closes inherited pipe writers before the bounded drains.
        #[cfg(unix)]
        process_group.terminate();

        // Bounded drain: grandchildren are now dead so EOF is imminent, but cap
        // defensively (mirrors handoff's 2s ceiling in bg.rs). A process that
        // somehow escaped the group kill just times out instead of hanging.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
        })
        .await;
        let (stdout, stderr) = {
            let st = state.lock().unwrap();
            if let Some(error) = st.output_limit_error() {
                return Ok(ToolOutput::err(error));
            }
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
mod tests;
