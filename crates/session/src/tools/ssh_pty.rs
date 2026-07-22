//! Persistent SSH sessions via local tmux. Provides `connect`, `send`, `read`,
//! `disconnect`, and `status` actions. Shell state (cwd, exports, background
//! jobs) persists across calls because each session is a real tmux pane running
//! `ssh -tt`. Interactive programs are rejected at the `send` layer.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Session registry (process-global, keyed by session_id for isolation)
// ---------------------------------------------------------------------------

struct SshSessionInfo {
    tmux_name: String,
    host: String,
}

static SSH_SESSIONS: LazyLock<Mutex<HashMap<String, SshSessionInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_session(session_id: &str) -> Option<(String, String)> {
    SSH_SESSIONS
        .lock()
        .unwrap()
        .get(session_id)
        .map(|i| (i.tmux_name.clone(), i.host.clone()))
}

// ---------------------------------------------------------------------------
// Command sanitisation — reject interactive programs
// ---------------------------------------------------------------------------

/// Binaries that are interactive TUIs / pagers — always blocked regardless
/// of arguments.
const ALWAYS_INTERACTIVE: &[&str] = &[
    // Editors
    "vim",
    "vi",
    "nvim",
    "neovim",
    "nano",
    "emacs",
    "gedit",
    // Process monitors
    "top",
    "htop",
    "btop",
    "atop",
    "bottom",
    "btm",
    "glances",
    // Pagers
    "less",
    "more",
    "man",
    "info",
    "bat",
    // Multiplexers (they take over the terminal)
    "tmux",
    "screen",
    // Debuggers
    "gdb",
    "lldb",
    "pdb",
    "ipdb",
    // File managers / git TUIs
    "ranger",
    "mc",
    "tig",
    "lazygit",
    "lazydocker",
    // Dialog / terminal UI builders
    "dialog",
    "whiptail",
    // Monitors that hold the terminal
    "watch",
    // Mail / chat clients
    "mutt",
    "neomutt",
    "irssi",
    "weechat",
];

/// Binaries that launch a REPL when invoked with no arguments. Allowed when
/// given `-c`, `-e`, a script path, or any other argument.
const BARE_INTERACTIVE: &[&str] = &[
    "python",
    "python3",
    "python2",
    "node",
    "irb",
    "pry",
    "mysql",
    "psql",
    "sqlite3",
    "redis-cli",
    "mongo",
    "mongosh",
    "bc",
    "ghci",
    "telnet",
    "ftp",
    "sftp",
];

fn strip_leading_sudo(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed
        .strip_prefix("sudo ")
        .or_else(|| trimmed.strip_prefix("doas "))
    {
        strip_leading_sudo(rest)
    } else {
        trimmed
    }
}

/// Returns true when `tok` looks like an environment-variable assignment
/// (`KEY=value`) that `env` consumes before the real command.
fn is_env_assignment(tok: &str) -> bool {
    if let Some(eq) = tok.find('=') {
        let key = &tok[..eq];
        !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    } else {
        false
    }
}

/// Strip wrapper commands (`env`, `exec`, `command`, `nohup`, `timeout`,
/// `strace`, `ltrace`, `perf`, `valgrind`, `nice`, `ionice`) that merely
/// delegate to the real program. This prevents trivial denylist bypasses like
/// `env vim`, `exec vim`, or `nohup top`.
///
/// Applies recursively so `env VAR=x exec sudo vim` is fully unwrapped.
/// For `env`, skips leading `KEY=value` assignments. For `timeout`, skips the
/// duration token. For tracing tools (`strace`/`ltrace`/`perf`/`valgrind`),
/// skips leading flag tokens (`-flag`).
fn strip_wrappers(cmd: &str) -> &str {
    let stripped = strip_leading_sudo(cmd);
    let first = stripped.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);

    match base {
        "env" => {
            let rest = stripped[first.len()..].trim_start();
            let mut pos = rest;
            while let Some(tok) = pos.split_whitespace().next() {
                if is_env_assignment(tok) {
                    pos = pos[tok.len()..].trim_start();
                } else {
                    break;
                }
            }
            strip_wrappers(pos)
        }
        "exec" | "command" | "nohup" | "nice" | "ionice" => {
            strip_wrappers(stripped[first.len()..].trim_start())
        }
        "timeout" => {
            // timeout takes a duration argument before the command.
            let rest = stripped[first.len()..].trim_start();
            if let Some(end) = rest.find(char::is_whitespace) {
                strip_wrappers(rest[end..].trim_start())
            } else {
                rest
            }
        }
        "strace" | "ltrace" | "perf" | "valgrind" => {
            let mut rest = stripped[first.len()..].trim_start();
            while let Some(tok) = rest.split_whitespace().next() {
                if tok.starts_with('-') {
                    rest = rest[tok.len()..].trim_start();
                } else {
                    break;
                }
            }
            strip_wrappers(rest)
        }
        _ => stripped,
    }
}

fn cmd_base(cmd: &str) -> &str {
    let first = cmd.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first)
}

/// Returns `Some(reason)` when the command launches an interactive program.
fn is_interactive_command(cmd: &str) -> Option<String> {
    let stripped = strip_wrappers(cmd);
    let base = cmd_base(stripped);
    let has_args = stripped.split_whitespace().count() > 1;

    if ALWAYS_INTERACTIVE.contains(&base) {
        return Some(format!(
            "'{base}' opens an interactive TUI that cannot be driven via send-keys. \
             Use a non-interactive alternative (sed, awk, head, grep, etc.)."
        ));
    }
    if BARE_INTERACTIVE.contains(&base) && !has_args {
        return Some(format!(
            "'{base}' without arguments launches an interactive REPL. \
             Pass a script (-c), command (-e), or filename to run non-interactively."
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// tmux helpers
// ---------------------------------------------------------------------------

fn sanitize_tmux_name(host: &str) -> String {
    let sanitized: String = host
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("oc-ssh-{sanitized}")
}

// ---------------------------------------------------------------------------
// Input validation — prevent shell injection through host / port / key_path
// ---------------------------------------------------------------------------

/// Characters that are dangerous when a value is interpolated into a shell
/// command string (as `host`, `port`, and `key_path` are for the tmux
/// `new-session` command). Rejecting these blocks command injection.
const SHELL_DANGEROUS: &[char] = &[
    ';', '|', '&', '`', '$', '(', ')', '{', '}', '<', '>', '\\', '"', '\'', '!', '*', '?', '[',
    ']', '\n', '\r', ' ', '\t',
];

/// Validate that `port` is a numeric value in the valid TCP range (1–65535).
fn validate_port(port: &str) -> Result<(), String> {
    match port.parse::<u16>() {
        Ok(p) if p > 0 => Ok(()),
        _ => Err(format!(
            "Invalid port '{port}'. Must be a number between 1 and 65535."
        )),
    }
}

/// Validate that `value` contains no shell metacharacters, preventing
/// injection through `host` or `key_path` when they are interpolated into
/// the tmux shell command.
fn validate_no_shell_injection(label: &str, value: &str) -> Result<(), String> {
    if value.chars().any(|c| SHELL_DANGEROUS.contains(&c)) {
        return Err(format!(
            "Invalid {label}: contains characters that are unsafe in a shell \
             command. Only simple hostname and path characters are allowed."
        ));
    }
    Ok(())
}

async fn capture_pane(tmux_name: &str, lines: u32) -> Option<String> {
    let lines_str = format!("-{}", lines);
    let output = tokio::process::Command::new("tmux")
        .args(["capture-pane", "-t", tmux_name, "-p", "-S", &lines_str])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

async fn tmux_session_exists(tmux_name: &str) -> bool {
    tokio::process::Command::new("tmux")
        .args(["has-session", "-t", tmux_name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Action handlers
// ---------------------------------------------------------------------------

async fn do_connect(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let host = input.get("host").and_then(|v| v.as_str()).unwrap_or("");
    if host.is_empty() {
        return Ok(ToolOutput::err(
            "Missing required parameter: host (e.g. 'user@1.2.3.4' or 'host').",
        ));
    }
    let tmux_name = sanitize_tmux_name(host);

    // Already connected to the same host?
    if let Some((existing_name, existing_host)) = get_session(&ctx.session_id) {
        if existing_name == tmux_name {
            return Ok(ToolOutput::ok(format!(
                "Already connected to {existing_host} (tmux session '{tmux_name}')."
            )));
        }
        // Different host — disconnect the old one first.
        let _ = tokio::process::Command::new("tmux")
            .args(["kill-session", "-t", &existing_name])
            .output()
            .await;
        SSH_SESSIONS.lock().unwrap().remove(&ctx.session_id);
    }

    // Validate inputs to prevent shell injection through host/port/key_path.
    if let Err(msg) = validate_no_shell_injection("host", host) {
        return Ok(ToolOutput::err(msg));
    }
    if let Some(port) = input
        .get("port")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Err(msg) = validate_port(port) {
            return Ok(ToolOutput::err(msg));
        }
        if let Err(msg) = validate_no_shell_injection("port", port) {
            return Ok(ToolOutput::err(msg));
        }
    }
    if let Some(key) = input
        .get("key_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Err(msg) = validate_no_shell_injection("key_path", key) {
            return Ok(ToolOutput::err(msg));
        }
    }

    // Build the ssh invocation.
    let mut ssh_cmd = String::from("ssh -tt");
    if let Some(port) = input
        .get("port")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        ssh_cmd.push_str(&format!(" -p {port}"));
    }
    if let Some(key) = input
        .get("key_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        ssh_cmd.push_str(&format!(" -i {key}"));
    }
    ssh_cmd.push(' ');
    ssh_cmd.push_str(host);

    let result = tokio::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", &tmux_name, &ssh_cmd])
        .output()
        .await;

    match result {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            return Ok(ToolOutput::err(format!(
                "Failed to create tmux session: {stderr}"
            )));
        }
        Err(_) => {
            return Ok(ToolOutput::err(
                "tmux binary not found. Install it: apt install tmux (or your package manager).",
            ));
        }
    }

    // Give SSH a moment to establish the connection.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Inject a non-interactive environment so pagers never block.
    let env_setup = "export TERM=dumb PAGER=cat GIT_PAGER=cat MANPAGER=cat LESS=FRXM";
    let _ = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", &tmux_name, "-l", env_setup])
        .output()
        .await;
    let _ = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", &tmux_name, "Enter"])
        .output()
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    SSH_SESSIONS.lock().unwrap().insert(
        ctx.session_id.clone(),
        SshSessionInfo {
            tmux_name: tmux_name.clone(),
            host: host.to_string(),
        },
    );

    Ok(ToolOutput::ok(format!(
        "Connected to {host} via tmux session '{tmux_name}'. \
         Shell state (cwd, exports) persists across sends."
    )))
}

async fn do_send(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if command.is_empty() {
        return Ok(ToolOutput::err("Missing required parameter: command."));
    }
    if let Some(reason) = is_interactive_command(command) {
        return Ok(ToolOutput::err(reason));
    }

    let (tmux_name, host) = match get_session(&ctx.session_id) {
        Some(info) => info,
        None => {
            return Ok(ToolOutput::err(
                "No SSH session connected. Use action='connect' first.",
            ))
        }
    };

    // Unique completion marker.
    let marker = format!(
        "__OC_DONE_{}__",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    // Send the command followed by the marker echo.
    let full_cmd = format!("{}; printf '\\n{}\\n'", command, marker);
    let _ = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", &tmux_name, "-l", &full_cmd])
        .output()
        .await;
    let _ = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", &tmux_name, "Enter"])
        .output()
        .await;

    // Poll for the marker (up to 30 s).
    let mut pane = String::new();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(output) = capture_pane(&tmux_name, 800).await {
            pane = output;
            if pane.contains(&marker) {
                break;
            }
        }
    }

    // Extract output: everything before the marker.
    let result = if let Some(pos) = pane.rfind(&marker) {
        pane[..pos].trim_end().to_string()
    } else {
        format!("[timeout: command may still be running]\n\n{pane}")
    };

    Ok(truncate_output(
        format!("[{host}]\n{result}"),
        ctx.max_output,
    ))
}

async fn do_read(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let lines = input.get("lines").and_then(|v| v.as_u64()).unwrap_or(200) as u32;

    let (tmux_name, host) = match get_session(&ctx.session_id) {
        Some(info) => info,
        None => {
            return Ok(ToolOutput::err(
                "No SSH session connected. Use action='connect' first.",
            ))
        }
    };

    match capture_pane(&tmux_name, lines).await {
        Some(output) => Ok(truncate_output(
            format!("[{host}]\n{output}"),
            ctx.max_output,
        )),
        None => Ok(ToolOutput::err(format!(
            "Failed to capture pane for tmux session '{tmux_name}'."
        ))),
    }
}

async fn do_disconnect(ctx: &ToolContext) -> Result<ToolOutput> {
    let (tmux_name, host) = match get_session(&ctx.session_id) {
        Some(info) => info,
        None => return Ok(ToolOutput::ok("No active SSH session.")),
    };
    let _ = tokio::process::Command::new("tmux")
        .args(["kill-session", "-t", &tmux_name])
        .output()
        .await;
    SSH_SESSIONS.lock().unwrap().remove(&ctx.session_id);
    Ok(ToolOutput::ok(format!("Disconnected from {host}.")))
}

async fn do_status(ctx: &ToolContext) -> Result<ToolOutput> {
    match get_session(&ctx.session_id) {
        Some((tmux_name, host)) => {
            let alive = tmux_session_exists(&tmux_name).await;
            if alive {
                Ok(ToolOutput::ok(format!(
                    "Connected to {host} (tmux session '{tmux_name}' is active)."
                )))
            } else {
                SSH_SESSIONS.lock().unwrap().remove(&ctx.session_id);
                Ok(ToolOutput::ok(format!(
                    "Session for {host} was found in registry but tmux session \
                     '{tmux_name}' no longer exists. Cleaned up."
                )))
            }
        }
        None => Ok(ToolOutput::ok("No SSH session registered.")),
    }
}

// ---------------------------------------------------------------------------
// Tool trait impl
// ---------------------------------------------------------------------------

pub struct SshPtyTool;

#[async_trait]
impl Tool for SshPtyTool {
    fn name(&self) -> &str {
        "ssh_pty"
    }
    fn description(&self) -> &str {
        "Persistent SSH via local tmux. Actions: connect (host, optional port/key_path), \
         send (command — runs remotely, output returned), read (snapshot current pane), \
         disconnect, status. Shell state persists across sends. Interactive programs \
         (vim/nano/top/REPL) are rejected."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "action".into(),
            serde_json::json!({
                "type": "string",
                "enum": ["connect", "send", "read", "disconnect", "status"],
                "description": "The operation to perform."
            }),
        );
        props.insert(
            "host".into(),
            json::prop_str("SSH target, e.g. 'user@1.2.3.4' or 'hostname'. Required for connect."),
        );
        props.insert("port".into(), json::prop_str("SSH port (default 22)."));
        props.insert(
            "key_path".into(),
            json::prop_str("Path to SSH private key (-i)."),
        );
        props.insert(
            "command".into(),
            json::prop_str("Command to run remotely (send action)."),
        );
        props.insert("lines".into(), serde_json::json!({"type": "integer", "description": "Number of scrollback lines for read (default 200)."}));
        json::object_schema(Value::Object(props), &["action"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "connect" => do_connect(&input, ctx).await,
            "send" => do_send(&input, ctx).await,
            "read" => do_read(&input, ctx).await,
            "disconnect" => do_disconnect(ctx).await,
            "status" => do_status(ctx).await,
            other => Ok(ToolOutput::err(format!(
                "Unknown action '{other}'. Use connect, send, read, disconnect, or status."
            ))),
        }
    }
}

#[cfg(test)]
#[path = "ssh_pty_tests.rs"]
mod tests;
