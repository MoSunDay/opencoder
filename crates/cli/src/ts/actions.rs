//! `opencode ts` actions: start / list / resume / cleanup, plus session seeding.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use opencoder_store::{SessionFilter, SessionListItem, SessionMeta, Store};

use crate::Cli;

use super::display::{abbreviate_path, format_ts, id8, ms_to_secs, preview_of, task_head};
use super::env::tmux_available;
use super::naming::{fresh_id, id_from_name, resolve_target, session_name};
use super::tmux::{attach, list_managed, session_exists, ManagedSession, tmux_bin};

/// `opencode ts`/`rs` (bare). When not forced-new:
/// - If `--session <id>` is set and live, attach it.
/// - Else if any live session exists, attach the most recent one.
/// - Else create a fresh session.
///
/// `--new` (`force_new`) skips reuse and always creates.
pub(crate) async fn ts_start(cli: &Cli, force_new: bool) -> Result<()> {
    if !tmux_available() {
        bail!(
            "tmux is not installed. Install it (e.g. `apt install tmux`), or run \
             `opencode tui` for a non-persistent session."
        );
    }
    if !force_new {
        if let Some(id) = &cli.session {
            let name = session_name(id);
            if session_exists(&name)? {
                return attach(&name);
            }
        } else if let Some(live) = list_managed()?.into_iter().next() {
            return attach(&live.name);
        }
    }
    let workdir = current_workdir(cli)?;
    let id = cli.session.clone().unwrap_or_else(fresh_id);
    ensure_session(&workdir, &id).await?;
    spawn_session(&workdir, &id)
}

/// Spawn `tmux new-session` running `<exe> tui --session <id> --workdir <wd>`.
/// Caller guarantees the tmux session name does NOT already exist.
fn spawn_session(workdir: &Path, id: &str) -> Result<()> {
    let name = session_name(id);
    if session_exists(&name)? {
        bail!("tmux session '{name}' already exists; use `opencode ts -r <id>` to resume");
    }
    let exe = std::env::current_exe().context("resolve opencoder executable")?;
    let mut cmd = Command::new(tmux_bin()?);
    cmd.arg("new-session")
        .arg("-s")
        .arg(&name)
        .arg("-c")
        .arg(workdir)
        .arg(exe.as_os_str())
        .arg("tui")
        .arg("--session")
        .arg(id)
        .arg("--workdir")
        .arg(workdir);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = cmd.status().context("spawn tmux new-session")?;
    if !status.success() {
        bail!("tmux new-session failed (exit {:?})", status.code());
    }
    Ok(())
}

/// Three-way tmux liveness for a Store session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmuxState {
    Attached,
    Detached,
    Dead,
}

/// True when `path` is the stopped-sentinel, so stopped rows sort after live
/// ones regardless of ASCII ordering (`(` would otherwise precede `~`).
fn is_stopped(path: &str) -> bool {
    path == "(stopped)"
}

/// Classify a Store session's tmux state given the live tmux id map.
fn classify(id: &str, tmux_by_id: &HashMap<String, &ManagedSession>) -> TmuxState {
    match tmux_by_id.get(id) {
        Some(m) if m.attached != 0 => TmuxState::Attached,
        Some(_) => TmuxState::Detached,
        None => TmuxState::Dead,
    }
}

/// `opencode ts -l` -- Store-first unified panel. Lists ALL sessions (live +
/// stopped) from the store, annotating each with its tmux state.
///
/// Columns: `marker id8 created-ago workdir task-head`.
/// Sorting: by workdir path ascending, then by creation time descending
/// (newest first within each path group).
pub(crate) async fn ts_list(cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let store = open_store_for(&workdir).await?;
    let sessions = store
        .list_sessions(&SessionFilter {
            limit: 500,
            ..Default::default()
        })
        .await?;

    let tmux = list_managed()?;
    let tmux_by_id: HashMap<String, &ManagedSession> = tmux
        .iter()
        .filter_map(|m| m.id().map(|i| (i.to_string(), m)))
        .collect();

    let mut rows: Vec<(String, i64, &SessionListItem, TmuxState)> = sessions
        .iter()
        .map(|s| {
            let st = classify(&s.id, &tmux_by_id);
            let path_display = match tmux_by_id.get(&s.id) {
                Some(m) => abbreviate_path(&m.pane_path),
                None => "(stopped)".to_string(),
            };
            (path_display, s.created_at, s, st)
        })
        .collect();

    rows.sort_by(|a, b| {
        is_stopped(&a.0)
            .cmp(&is_stopped(&b.0))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| b.1.cmp(&a.1))
    });

    if rows.is_empty() {
        println!("(no sessions)");
        return Ok(());
    }
    for (path_display, _, s, st) in &rows {
        let marker = match st {
            TmuxState::Attached => "*",
            TmuxState::Detached => "\u{00b7}",
            TmuxState::Dead => " ",
        };
        let raw = preview_of(&s.preview, s.title.as_deref());
        let task = if raw.trim().is_empty() {
            "(no task yet)".to_string()
        } else {
            task_head(raw, 20)
        };
        println!(
            "{} {:<10} {:<9} {:<16} {}",
            marker,
            id8(&s.id),
            format_ts(ms_to_secs(s.created_at)),
            path_display,
            task
        );
    }
    println!();
    println!("* attached  \u{00b7} live(detached)  (space) stopped");
    println!("resume: opencode ts -r <id>   new: opencode ts --new   clean: opencode ts -c");
    Ok(())
}

/// `opencode ts -r <id>` -- resume/attach a live session, or cold-start a
/// stopped one from Store history.
pub(crate) async fn ts_resume(cli: &Cli, target: &str) -> Result<()> {
    let resolved = resolve_target(target);
    if session_exists(&resolved)? {
        return attach(&resolved);
    }
    // Dead: cold-start if the Store has this session.
    let id = id_from_name(&resolved).unwrap_or_else(|| target.trim());
    let workdir = current_workdir(cli)?;
    let store = open_store_for(&workdir).await?;
    if store.get_session(id).await?.is_none() {
        bail!(
            "no session matching `{}` (not in tmux, not in store). Run `opencode ts -l` to list.",
            target
        );
    }
    spawn_session(&workdir, id)
}

/// `opencode ts -c` -- delete stopped sessions (in Store but not in tmux).
pub(crate) async fn ts_cleanup(cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let store = open_store_for(&workdir).await?;
    let sessions = store
        .list_sessions(&SessionFilter {
            limit: 500,
            ..Default::default()
        })
        .await?;

    let tmux = list_managed()?;
    let live_ids: HashSet<&str> = tmux.iter().filter_map(|m| m.id()).collect();

    let mut removed = 0u32;
    for s in &sessions {
        if !live_ids.contains(s.id.as_str()) {
            store.delete_session(&s.id).await?;
            removed += 1;
        }
    }
    if removed == 0 {
        println!("no stopped sessions to clean up.");
    } else {
        println!("removed {removed} stopped session(s).");
    }
    Ok(())
}

async fn ensure_session(workdir: &Path, id: &str) -> Result<()> {
    let store = open_store_for(workdir).await?;
    if store.get_session(id).await?.is_some() {
        return Ok(());
    }
    let now = opencoder_core::message::now_ms();
    store
        .create_session(&SessionMeta {
            id: id.to_string(),
            title: None,
            agent: None,
            model: None,
            workdir_hash: None,
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
        })
        .await
        .context("seed session for tmux")?;
    Ok(())
}

async fn open_store_for(workdir: &Path) -> Result<opencoder_store::LibsqlStore> {
    let wd = PathBuf::from(workdir);
    crate::session_cmd::open_store(&wd).await
}

fn current_workdir(cli: &Cli) -> Result<PathBuf> {
    if let Some(w) = &cli.workdir {
        return Ok(w.clone());
    }
    std::env::current_dir().context("get current dir")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_managed(id: &str, attached: u8) -> ManagedSession {
        ManagedSession {
            name: session_name(id),
            tmux_id: "$0".into(),
            created: 0,
            attached,
            pane_path: "/root/proj".into(),
        }
    }

    #[test]
    fn classify_three_states() {
        let m1 = mk_managed("01AAA", 1);
        let m2 = mk_managed("02BBB", 0);
        let map: HashMap<String, &ManagedSession> = [
            ("01AAA".to_string(), &m1),
            ("02BBB".to_string(), &m2),
        ]
        .into_iter()
        .collect();

        assert_eq!(classify("01AAA", &map), TmuxState::Attached);
        assert_eq!(classify("02BBB", &map), TmuxState::Detached);
        assert_eq!(classify("NOTHERE", &map), TmuxState::Dead);
    }

    #[test]
    fn sort_by_path_then_created_desc() {
        let mut rows = [
            ("~/projB".to_string(), 100i64),
            ("~/projA".to_string(), 300),
            ("~/projA".to_string(), 200),
            ("(stopped)".to_string(), 50),
        ];
        rows.sort_by(|a, b| {
            is_stopped(&a.0)
                .cmp(&is_stopped(&b.0))
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| b.1.cmp(&a.1))
        });
        assert_eq!(rows[0], ("~/projA".to_string(), 300));
        assert_eq!(rows[1], ("~/projA".to_string(), 200));
        assert_eq!(rows[2], ("~/projB".to_string(), 100));
        assert_eq!(rows[3], ("(stopped)".to_string(), 50));
    }

    #[test]
    fn now_ms_is_milliseconds() {
        let t = opencoder_core::message::now_ms();
        assert!(t > 1_000_000_000_000, "now_ms should be in milliseconds, got {t}");
    }
}
