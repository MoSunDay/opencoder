//! tmux process plumbing + the managed-session data model.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use super::env::which_tmux;
use super::naming::id_from_name;

pub(crate) fn tmux_bin() -> Result<PathBuf> {
    which_tmux().ok_or_else(|| {
        anyhow!(
            "tmux is not installed. Install it (e.g. `apt install tmux`) or use \
             `opencode tui` for a non-persistent session."
        )
    })
}

/// Run a tmux command inheriting stdio (used for attach/switch which take over
/// the terminal).
fn tmux_inherit(args: &[&str]) -> Result<()> {
    let status = Command::new(tmux_bin()?)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("spawn tmux")?;
    if !status.success() {
        bail!("tmux {} failed", args.join(" "));
    }
    Ok(())
}

/// Does a tmux session exist for `target` (already resolved)? Returns false
/// (not an error) when there is no tmux server at all.
pub(crate) fn session_exists(target: &str) -> Result<bool> {
    let status = Command::new(tmux_bin()?)
        .args(["has-session", "-t", target])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn tmux has-session")?;
    Ok(status.success())
}

/// One managed tmux session observed via `list-sessions`.
#[derive(Debug, Clone)]
pub(crate) struct ManagedSession {
    /// tmux session name, e.g. `opencode-01H...`.
    pub name: String,
    /// tmux global id, e.g. `$3`.
    pub tmux_id: String,
    /// Creation time, unix seconds.
    pub created: i64,
    /// 1 if a client is attached, else 0.
    pub attached: u8,
    /// Current working directory of the session's active pane (absolute).
    pub pane_path: String,
}

impl ManagedSession {
    pub fn id(&self) -> Option<&str> {
        id_from_name(&self.name)
    }
}

/// Parse one `tmux list-sessions -F` line (tab-separated
/// `name\tid\tcreated\tattached`). Returns `None` for unmanaged/malformed lines.
pub(crate) fn parse_list_line(line: &str) -> Option<ManagedSession> {
    let mut it = line.split('\t');
    let name = it.next()?.trim().to_string();
    id_from_name(&name)?;
    let tmux_id = it.next()?.trim().to_string();
    let created = it.next()?.trim().parse().ok()?;
    let attached = it.next()?.trim().parse().ok()?;
    let pane_path = it.next().map(|x| x.trim().to_string()).unwrap_or_default();
    Some(ManagedSession {
        tmux_id,
        created,
        attached,
        pane_path,
        name,
    })
}

/// All managed (`opencode-*`) tmux sessions, newest first. Returns an empty vec
/// (not an error) when tmux is absent or has no server / no managed sessions.
pub(crate) fn list_managed() -> Result<Vec<ManagedSession>> {
    let bin = match which_tmux() {
        Some(b) => b,
        None => return Ok(Vec::new()),
    };
    let out = Command::new(&bin)
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_id}\t#{session_created}\t#{session_attached}\t#{pane_current_path}",
        ])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(_) => return Ok(Vec::new()),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("no server running")
            || err.contains("no sessions")
            || err.contains("error connecting to")
        {
            return Ok(Vec::new());
        }
        bail!("tmux list-sessions failed: {}", err.trim());
    }
    let mut items: Vec<ManagedSession> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_list_line)
        .collect();
    items.sort_by_key(|m| std::cmp::Reverse(m.created));
    Ok(items)
}

/// Attach to (or switch-client within tmux to) `target` (already resolved).
pub(crate) fn attach(target: &str) -> Result<()> {
    if super::env::inside_tmux() {
        tmux_inherit(&["switch-client", "-t", target])
    } else {
        tmux_inherit(&["attach-session", "-t", target])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_line_managed_and_unmanaged() {
        let line = "opencode-01HZ\t$2\t1700000000\t1\t/root/proj";
        let m = parse_list_line(line).expect("managed line parses");
        assert_eq!(m.name, "opencode-01HZ");
        assert_eq!(m.tmux_id, "$2");
        assert_eq!(m.created, 1700000000);
        assert_eq!(m.attached, 1);
        assert_eq!(m.pane_path, "/root/proj");
        assert_eq!(m.id(), Some("01HZ"));

        // Unmanaged name -> None even if columns are well-formed.
        assert!(parse_list_line("vim-session\t$1\t1\t0\t/x").is_none());
        assert!(parse_list_line("garbage no tabs").is_none());
        assert!(parse_list_line("opencode-x\t$1\tnotnum\t0\t/x").is_none());
        // Missing the path column is tolerated (defaults to empty).
        let m2 = parse_list_line("opencode-AB\t$9\t5\t0").expect("no-path ok");
        assert_eq!(m2.pane_path, "");
    }
}
