use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

use super::bg::{drain_output, handoff, output_path, BgState, OutputStream};

pub mod host;
pub mod powershell;
#[path = "../bash/process_group.rs"]
pub(crate) mod process_group;
mod readonly;
#[cfg(not(windows))]
#[path = "../bash/timeout.rs"]
mod timeout;

use process_group::ProcessGroupGuard;

#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 130;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_SECS: u64 = 1;

#[cfg(not(test))]
pub(crate) const BASH_TIMEOUT_DISPLAY_SECS: u64 = 120;
#[cfg(test)]
pub(crate) const BASH_TIMEOUT_DISPLAY_SECS: u64 = 1;

#[cfg(not(test))]
const _: () = assert!(BASH_TIMEOUT_DISPLAY_SECS < BASH_TIMEOUT_SECS);

pub(crate) const BASH_TIMEOUT_MARKER: &str = if cfg!(windows) {
    "[powershell-timeout:"
} else {
    "[bash-timeout:"
};

pub struct ShellTool;

fn script_with_tools_path(command: &str, tools_path: Option<&str>) -> String {
    match tools_path {
        Some(joined) if !joined.is_empty() && !joined.contains('"') && !joined.contains('$') => {
            format!("export PATH=\"{joined}\":$PATH\n{command}")
        }
        _ => command.to_string(),
    }
}

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
impl Tool for ShellTool {
    fn name(&self) -> &str {
        opencoder_core::platform::shell::tool_name()
    }
    fn description(&self) -> &str {
        if cfg!(windows) {
            "Executes a PowerShell 7 command in the session working directory and returns stdout+stderr. Use PowerShell syntax for git, builds, tests and scripts. Commands run non-interactively."
        } else {
            "Executes a bash command in the session working directory and returns stdout+stderr. Use for git, builds, tests, running scripts. Commands run non-interactively."
        }
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "command".into(),
            json::prop_str("The command to execute in the host shell."),
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
        #[cfg(not(windows))]
        let resolved = timeout::resolve(command, BASH_TIMEOUT_SECS, BASH_TIMEOUT_DISPLAY_SECS);
        #[cfg(windows)]
        let resolved = WindowsTimeout {
            command,
            timeout_secs: BASH_TIMEOUT_SECS,
            display_secs: BASH_TIMEOUT_DISPLAY_SECS,
        };
        if resolved.command.trim().is_empty() {
            return Ok(ToolOutput::err("empty command"));
        }
        let workdir = input
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let script = if cfg!(windows) {
            powershell::script(resolved.command)
        } else {
            script_with_tools_path(resolved.command, ctx.tools_path.as_deref())
        };
        let program = host::program().await?;

        let supervised = crate::process::command(&program)?;
        let (mut cmd, lease) = match supervised {
            Some((command, lease)) => (command, Some(lease)),
            None => (tokio::process::Command::new(&program), None),
        };
        let supervised = lease.is_some();
        host::arguments(&mut cmd, &script);
        cmd.current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(ctx.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        #[cfg(windows)]
        if let Some(paths) = &ctx.tools_path {
            let inherited = ctx
                .extra_env
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
            cmd.env("PATH", format!("{paths};{inherited}"));
        }
        crate::process::configure_owned_command(&mut cmd, supervised);

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

        let mut child = cmd.spawn()?;
        let pid = match child.id() {
            Some(p) => p,
            None => return Ok(ToolOutput::err("failed to get child pid")),
        };
        let pgid = pid as i32;
        let mut process_group = if let Some(lease) = lease {
            ProcessGroupGuard::registered_supervised(
                pid,
                ctx.session_id.clone(),
                lease.spawned(child.id())?,
            )?
        } else {
            ProcessGroupGuard::registered(pid, pgid, ctx.session_id.clone())
        };

        let state = Arc::new(Mutex::new(BgState::new()));

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
                        is_error: false,
                        images: vec![],
                    });
            }
        };
        process_group.terminate();

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

#[cfg(windows)]
struct WindowsTimeout<'a> {
    command: &'a str,
    timeout_secs: u64,
    display_secs: u64,
}

#[cfg(all(test, windows))]
mod windows_tests;
