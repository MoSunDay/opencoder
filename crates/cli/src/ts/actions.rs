//! `opencode ts` actions: start / list / resume, plus session seeding.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use opencoder_store::{SessionFilter, SessionListItem, SessionMeta, Store};

use crate::Cli;

use super::display::{abbreviate_path, format_ts, now_secs, task_head};
use super::env::tmux_available;
use super::naming::{fresh_id, resolve_target, session_name};
use super::tmux::{attach, list_managed, session_exists, tmux_bin};

/// `opencode ts` (no flags). See module docs for the auto-reattach rule.
/// Caller (`main.rs`) diverts the already-inside-tmux case before calling this.
pub(crate) async fn ts_start(cli: &Cli, force_new: bool) -> Result<()> {
    if !tmux_available() {
        bail!(
            "tmux is not installed. Install it (e.g. `apt install tmux`), or run \
             `opencode tui` for a non-persistent session."
        );
    }
    // An explicit --session always means "fresh tmux for this session".
    if cli.session.is_some() {
        return start_new(cli).await;
    }
    let managed = list_managed()?;
    if !force_new && managed.len() == 1 {
        eprintln!(
            "ts: reattaching the single managed session {}",
            managed[0].name
        );
        return attach(&managed[0].name);
    }
    if !force_new && managed.len() > 1 {
        eprintln!(
            "ts: {} managed sessions already exist -- resume one with `ts -r <id>` \
             or start another with `--new`:",
            managed.len()
        );
        for m in &managed {
            eprintln!("  {}  ({})", m.name, m.tmux_id);
        }
    }
    start_new(cli).await
}

/// Create and attach a managed tmux session running the TUI. Blocks until the
/// client detaches; the TUI keeps running inside tmux.
async fn start_new(cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let id = match &cli.session {
        Some(s) => s.clone(),
        None => fresh_id(),
    };
    ensure_session(&workdir, &id).await?;

    let name = session_name(&id);
    let exe = std::env::current_exe().context("resolve opencoder executable")?;
    let mut cmd = Command::new(tmux_bin()?);
    cmd.arg("new-session")
        .arg("-s")
        .arg(&name)
        .arg("-c")
        .arg(&workdir)
        .arg(exe.as_os_str())
        .arg("tui")
        .arg("--session")
        .arg(&id)
        .arg("--workdir")
        .arg(&workdir);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = cmd.status().context("spawn tmux new-session")?;
    if !status.success() {
        bail!("tmux new-session failed (exit {:?})", status.code());
    }
    Ok(())
}

/// `opencode ts -l` -- list managed sessions with `/task`-style enrichment.
///
/// Columns: `* tmux-name | tmux-id | started | workdir | task`. `workdir` is the
/// tmux pane's current path (`#{pane_current_path}`) home-abbreviated; `task`
/// is the store's latest `/task` preview truncated to its first 10 characters.
pub(crate) async fn ts_list(cli: &Cli) -> Result<()> {
    let managed = list_managed()?;
    if managed.is_empty() {
        println!("(no managed ts sessions)");
        return Ok(());
    }
    let workdir = current_workdir(cli)?;
    let by_id: HashMap<String, SessionListItem> = match open_store_for(&workdir).await {
        Ok(store) => store
            .list_sessions(&SessionFilter {
                limit: 500,
                ..Default::default()
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|it| (it.id.clone(), it))
            .collect(),
        Err(_) => HashMap::new(),
    };

    for m in &managed {
        let marker = if m.attached != 0 { "*" } else { " " };
        let workdir = abbreviate_path(&m.pane_path);
        let task = match m.id().and_then(|id| by_id.get(id)) {
            Some(item) => {
                let raw = if !item.preview.trim().is_empty() {
                    item.preview.as_str()
                } else {
                    item.title.as_deref().unwrap_or("")
                };
                if raw.trim().is_empty() {
                    "(no task yet)".to_string()
                } else {
                    task_head(raw, 10)
                }
            }
            None => "(store not in this workdir)".to_string(),
        };
        println!(
            "{} {:<26} {:<5} {:<9} {:<22} {}",
            marker,
            m.name,
            m.tmux_id,
            format_ts(m.created),
            workdir,
            task
        );
    }
    println!(
        "\n* = attached    columns: tmux-name | tmux-id | started | workdir | task(first 10 chars)"
    );
    println!("resume: opencode ts -r <name|id>");
    Ok(())
}

/// `opencode ts -r <id>` -- resume/attach a managed session.
pub(crate) fn ts_resume(target: &str) -> Result<()> {
    let resolved = resolve_target(target);
    if !session_exists(&resolved)? {
        bail!(
            "no tmux session matching `{}` (tried `{}`). Run `opencode ts -l` to list.",
            target,
            resolved
        );
    }
    attach(&resolved)
}

async fn ensure_session(workdir: &Path, id: &str) -> Result<()> {
    let store = open_store_for(workdir).await?;
    if store.get_session(id).await?.is_some() {
        return Ok(());
    }
    let now = now_secs();
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
