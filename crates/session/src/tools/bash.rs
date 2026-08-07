use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::io::AsyncReadExt;

use super::bg::{handoff, output_path, register, unregister, BgState};
#[cfg(test)]
use super::bg::{kill_all, list, test_registry_mutex};

/// Real enforced foreground deadline for a bash command (seconds). When
/// exceeded the command is moved to the background
/// (see [`super::bg::handoff`]) instead of being killed — long-running
/// builds keep going and their output lands in a temp file the model can read
/// later. The runner keeps bash exempt from its own 600 s leaf-tool safety net
/// (`runner::execute`) so the two deadlines never race; bash has its own
/// shorter internal timeout.
///
/// NOTE: the number *shown to the model* in handoff/killed messages is
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

/// Model-visible timeout (seconds): the value interpolated into the timeout
/// handoff/killed messages the tool returns to the model. Deliberately *lower*
/// than the real enforced deadline ([`BASH_TIMEOUT_SECS`]); see that constant's
/// doc comment for the buffer rationale. Same 1 s override under `cfg(test)`.
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

/// Lower bound a `timeout N;` override prefix may resolve to (seconds).
/// Prevents the model from shrinking the foreground deadline below a sane
/// floor. In production this is 120 s; under `cfg(test)` it drops to 1 s so the
/// override path is exercisable without a 120 s wait (mirroring how
/// [`BASH_TIMEOUT_SECS`] itself is cfg-gated).
#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_MIN_SECS: u64 = 120;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_MIN_SECS: u64 = 1;

/// Upper bound a `timeout N;` override may resolve to (seconds). Caps an
/// otherwise unbounded model-authored deadline. Identical in tests and prod.
pub(crate) const BASH_TIMEOUT_MAX_SECS: u64 = 600;


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

/// Parse a leading `timeout <secs>;` override prefix with a pure string scan
/// (no regex). Recognises: the keyword `timeout`, one or more ASCII whitespace,
/// one or more ASCII digits (value >= 1), optional whitespace, then a `;`. On a
/// match returns `(parsed_seconds, command_after_the_semicolon)`; otherwise
/// `None`.
///
/// Only a *prefix* is matched, so an embedded occurrence such as
/// `echo "timeout 5"` never triggers. Time suffixes (`5m`, `2h`) are not
/// supported -- the value is bare seconds, matching the unit of the internal
/// deadline. The returned value is **unclamped**; callers apply [`clamp_timeout`]
/// to keep it within `[BASH_TIMEOUT_MIN_SECS, BASH_TIMEOUT_MAX_SECS]`.
fn parse_command_timeout(command: &str) -> Option<(u64, &str)> {
    let s = command.strip_prefix("timeout")?;
    let bytes = s.as_bytes();
    let mut i = 0;
    // Require >= 1 whitespace between the keyword and the number.
    let ws_start = i;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    if i == ws_start {
        return None;
    }
    // Collect the digit run.
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    let digits = &s[digits_start..i];
    // Skip optional whitespace between the number and the semicolon.
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    // A semicolon must immediately follow (the override ends here).
    if i >= bytes.len() || bytes[i] != b';' {
        return None;
    }
    // `digits` is a non-empty ASCII-digit slice, so parsing cannot fail in
    // practice -- guard defensively regardless.
    let secs = digits.parse::<u64>().ok()?;
    if secs == 0 {
        return None;
    }
    Some((secs, &s[i + 1..]))
}

/// Clamp a model-authored timeout override into the allowed band
/// `[BASH_TIMEOUT_MIN_SECS, BASH_TIMEOUT_MAX_SECS]`.
fn clamp_timeout(raw: u64) -> u64 {
    raw.clamp(BASH_TIMEOUT_MIN_SECS, BASH_TIMEOUT_MAX_SECS)
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
        // Silent per-command timeout override: a leading `timeout <secs>;`
        // prefix (parsed by pure string match -- NOT exposed in the schema)
        // widens or narrows both the real foreground deadline and the number
        // shown to the model, clamped to
        // [BASH_TIMEOUT_MIN_SECS, BASH_TIMEOUT_MAX_SECS]. The prefix is stripped
        // from the command actually run so GNU `timeout` is never invoked -- it
        // would error with exit 125 / stderr noise when given only a duration
        // and no subcommand.
        let (timeout_secs, display_secs, run_cmd) = match parse_command_timeout(command) {
            Some((raw, rest)) => {
                if rest.trim().is_empty() {
                    return Ok(ToolOutput::err("empty command"));
                }
                let secs = clamp_timeout(raw);
                (secs, secs, rest)
            }
            None => (BASH_TIMEOUT_SECS, BASH_TIMEOUT_DISPLAY_SECS, command),
        };
        let workdir = input
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-lc")
            .arg(run_cmd)
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
            Duration::from_secs(timeout_secs),
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
                            "{BASH_TIMEOUT_MARKER} command timed out after {display_secs}s \u{2014} moved to background]\n\
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
                            "{BASH_TIMEOUT_MARKER} command timed out after {display_secs}s \u{2014} killed]",
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

        // Kill the process group so any grandchildren that inherited the pipe
        // write-ends die and the drain tasks reach EOF. Without this, a command
        // like `cmd &` leaves a grandchild holding the pipe open and the drain
        // awaits below hang forever — and bash is exempt from the runner's
        // safety net (None deadline), so nothing breaks the hang. Mirrors the
        // handoff supervisor's group kill in bg.rs.
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGKILL);
        }

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

    /// A command that backgrounds a grandchild (e.g. `cmd &`) which inherits
    /// the stdout pipe returns promptly on natural completion: the group kill
    /// reaps the leaked grandchild so the drain tasks reach EOF instead of
    /// hanging forever (bash has no runner deadline, so nothing else breaks it).
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_returns_when_grandchild_leaks_pipe() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        // `sleep 30 &` spawns a process that inherits stdout; bash exits at
        // once (wait returns Ok) but the grandchild keeps the pipe open.
        let input = json!({"command": "echo done; sleep 30 &"});
        let result =
            tokio::time::timeout(Duration::from_secs(10), tool.execute(input, &ctx())).await;
        assert!(
            result.is_ok(),
            "bash should return within 10s even when a grandchild holds the pipe"
        );
        let out = result.unwrap().unwrap();
        assert!(
            out.content.contains("done"),
            "expected the command's output, got: {}",
            out.content
        );
        kill_all();
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

    /// The handoff message reports the *display* timeout, not the real one.
    /// Guards against a "fix" that switches the message back to the 130 s
    /// constant. (Under cfg(test) both are 1 s, so this primarily pins the code
    /// path to the display constant; the 120<130 invariant is enforced at
    /// compile time by the `const _: () = assert!(...)` in non-test builds.)
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_timeout_message_uses_display_constant() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        let input = json!({"command": "sleep 3"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(
            out.content
                .contains(&format!("after {BASH_TIMEOUT_DISPLAY_SECS}s")),
            "handoff message must quote the display constant: {}",
            out.content
        );
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
    /// --- timeout-override prefix parsing ---

    #[test]
    fn parse_command_timeout_basic() {
        let (secs, rest) = parse_command_timeout("timeout 5; echo hi").expect("basic prefix");
        assert_eq!(secs, 5);
        assert_eq!(rest, " echo hi");
    }

    #[test]
    fn parse_command_timeout_extra_whitespace() {
        // Multiple spaces after the keyword...
        let (secs, rest) = parse_command_timeout("timeout   10; cmd").unwrap();
        assert_eq!(secs, 10);
        assert_eq!(rest, " cmd");
        // ...and between the number and the semicolon.
        let (secs2, rest2) = parse_command_timeout("timeout 7  ; cmd").unwrap();
        assert_eq!(secs2, 7);
        assert_eq!(rest2, " cmd");
        // A tab counts as whitespace too.
        let (secs3, _) = parse_command_timeout("timeout\t3; cmd").unwrap();
        assert_eq!(secs3, 3);
    }

    #[test]
    fn parse_command_timeout_large_number_unclamped() {
        // parse returns the raw value; clamping is the caller's job.
        let (secs, _) = parse_command_timeout("timeout 99999; cmd").unwrap();
        assert_eq!(secs, 99999);
    }

    #[test]
    fn parse_command_timeout_rejects_invalid() {
        // no prefix at all
        assert!(parse_command_timeout("echo hi").is_none());
        // not a prefix (keyword appears mid-command)
        assert!(parse_command_timeout("echo timeout 5; x").is_none());
        // no whitespace after keyword
        assert!(parse_command_timeout("timeout5; cmd").is_none());
        // bare keyword, no number
        assert!(parse_command_timeout("timeout; cmd").is_none());
        // non-digit where a number is expected
        assert!(parse_command_timeout("timeout abc; cmd").is_none());
        // number but no terminating semicolon
        assert!(parse_command_timeout("timeout 60").is_none());
        // zero is not a valid override
        assert!(parse_command_timeout("timeout 0; cmd").is_none());
        // empty command
        assert!(parse_command_timeout("").is_none());
    }

    #[test]
    fn clamp_timeout_stays_in_band() {
        // The cfg(test) MIN is 1 s (kept low so the integration tests below can
        // exercise the override quickly); MAX is 600 s in both builds.
        assert_eq!(clamp_timeout(0), BASH_TIMEOUT_MIN_SECS);
        assert_eq!(clamp_timeout(BASH_TIMEOUT_MIN_SECS), BASH_TIMEOUT_MIN_SECS);
        assert_eq!(clamp_timeout(300), 300);
        assert_eq!(clamp_timeout(BASH_TIMEOUT_MAX_SECS), BASH_TIMEOUT_MAX_SECS);
        assert_eq!(clamp_timeout(u64::MAX / 2), BASH_TIMEOUT_MAX_SECS);
        // The *production* floor/ceiling (120/600) are honoured for explicit
        // bounds, regardless of cfg:
        assert_eq!(50u64.clamp(120, 600), 120);
        assert_eq!(900u64.clamp(120, 600), 600);
        assert_eq!(300u64.clamp(120, 600), 300);
    }

    /// `timeout 2; sleep 5` resolves to a 2 s deadline (override=2; the MIN is
    /// 1 s under cfg(test)); `sleep 5` exceeds it, triggering the background
    /// handoff. The message must report the *override* value (2 s), proving both
    /// that the deadline was overridden and that the displayed number tracks `x`
    /// rather than the default 1 s.
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_timeout_override_triggers_handoff() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        let input = json!({"command": "timeout 2; sleep 5"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(
            !out.is_error,
            "timeout is not an error -- command still runs in background: {}",
            out.content
        );
        assert!(
            out.content.contains(BASH_TIMEOUT_MARKER),
            "expected timeout handoff marker: {}",
            out.content
        );
        assert!(
            out.content.contains("after 2s"),
            "handoff message must report the overridden deadline (2s), got: {}",
            out.content
        );
        kill_all();
    }

    /// `timeout 5; sleep 2; echo ok` resolves to a 5 s deadline. `sleep 2`
    /// exceeds the *default* test deadline (1 s) yet finishes well inside the
    /// 5 s override, so the command completes in the foreground: no handoff and
    /// the command's own output ("ok") is returned. This proves the override
    /// widens the deadline beyond the default rather than being a no-op.
    #[tokio::test]
    #[cfg(unix)]
    async fn bash_timeout_override_widens_deadline() {
        let _g = test_registry_mutex().lock().await;
        let tool = BashTool;
        let input = json!({"command": "timeout 5; sleep 2; echo ok"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(!out.is_error, "expected success, got: {}", out.content);
        assert!(
            out.content.contains("ok"),
            "expected the command's own output, got: {}",
            out.content
        );
        assert!(
            !out.content.contains(BASH_TIMEOUT_MARKER),
            "override (5s) should outlast sleep 2 -- no handoff: {}",
            out.content
        );
    }

    /// `timeout 7;` with nothing after it is an empty command once the prefix
    /// is stripped -- it must hit the empty-command error path, not spawn an
    /// empty shell.
    #[tokio::test]
    async fn bash_timeout_override_empty_rest_errors() {
        let tool = BashTool;
        let input = json!({"command": "timeout 7;"});
        let out = tool.execute(input, &ctx()).await.unwrap();
        assert!(out.is_error, "expected empty-command error: {}", out.content);
        assert!(
            out.content.contains("empty command"),
            "expected the empty-command error message, got: {}",
            out.content
        );
    }

}
