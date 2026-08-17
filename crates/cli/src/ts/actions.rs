//! `opencode ts` actions: start / list / resume / cleanup.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use opencoder_store::{LibsqlStore, Store, TsRecord, TsRegistry};

use crate::Cli;

use super::display::{abbreviate_path, format_ts, id8, preview_of, task_head};
use super::env::tmux_available;
use super::naming::{fresh_id, id_from_name, resolve_target, session_name};
use super::registry::{open_registry, register};
use super::tmux::{
    attach, current_session_name, kill_session, list_managed, session_exists, tmux_bin,
    ManagedSession,
};

/// `opencode ts`/`rs` (bare). A bare `ts`/`rs` **always creates a fresh
/// session**; it never reuses an existing live one. This avoids the surprising
/// case where running `ts` in repo B silently attached to a session created in
/// repo A. To resume an existing session use `ts -r <id>`.
///
/// The single reuse exception is `--session <id>`: a live instance is attached;
/// a globally registered stopped instance is cold-started in its recorded
/// workdir. Only an unknown explicit id is seeded in the current workdir.
pub(crate) async fn ts_start(cli: &Cli) -> Result<()> {
    if !tmux_available() {
        bail!(
            "tmux is not installed. Install it (e.g. `apt install tmux`), or run \
             `opencode tui` for a non-persistent session."
        );
    }
    // Attach only via the explicit `--session <id>` shortcut: if that exact
    // session is already live in tmux, reattach instead of spawning a clone.
    // A bare `ts` (no --session) always falls through to create a fresh one.
    let attach_name = match &cli.session {
        Some(id) => {
            let exists = session_exists(&session_name(id))?;
            explicit_attach_target(Some(id), exists)
        }
        None => explicit_attach_target(None, false),
    };
    if let Some(name) = attach_name {
        return attach(&name);
    }

    let registry = open_registry().await?;
    // `--session <id>` naming a globally registered stopped session cold-starts
    // it in its recorded workdir instead of seeding a fresh one here.
    if let Some(id) = &cli.session {
        if registry.get(id).await?.is_some() {
            return ts_resume(cli, id).await;
        }
    }
    let workdir = current_workdir(cli)?;
    let id = cli.session.clone().unwrap_or_else(fresh_id);
    register(&registry, &id, &workdir).await?;
    spawn_session(&workdir, &id)
}

/// Pure decision: a bare `ts` always creates a new session. The only attach
/// shortcut is an explicit `--session <id>` whose tmux session already exists.
/// Returns the name to attach to, if any.
///
/// `exists` reflects whether the tmux session named `opencode-<id>` is live.
pub(crate) fn explicit_attach_target(session_arg: Option<&str>, exists: bool) -> Option<String> {
    match session_arg {
        Some(id) if exists => Some(session_name(id)),
        _ => None,
    }
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
    let inside = super::env::inside_tmux();
    cmd.args(spawn_args(&exe, workdir, id, inside));
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = cmd.status().context("spawn tmux new-session")?;
    if !status.success() {
        bail!("tmux new-session failed (exit {:?})", status.code());
    }
    if inside {
        attach(&name)
    } else {
        Ok(())
    }
}

fn spawn_args(exe: &Path, workdir: &Path, id: &str, inside: bool) -> Vec<OsString> {
    let mut args = vec![OsString::from("new-session")];
    if inside {
        args.push(OsString::from("-d"));
    }
    args.extend([
        OsString::from("-s"),
        OsString::from(session_name(id)),
        OsString::from("-c"),
        workdir.as_os_str().to_owned(),
        exe.as_os_str().to_owned(),
        OsString::from("tui"),
        OsString::from("--session"),
        OsString::from(id),
        OsString::from("--workdir"),
        workdir.as_os_str().to_owned(),
    ]);
    args
}

/// Three-way tmux liveness for a registry session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmuxState {
    Attached,
    Detached,
    Dead,
}

/// Classify a session's tmux state given the live tmux id map.
fn classify(id: &str, tmux_by_id: &HashMap<String, &ManagedSession>) -> TmuxState {
    match tmux_by_id.get(id) {
        Some(m) if m.attached != 0 => TmuxState::Attached,
        Some(_) => TmuxState::Detached,
        None => TmuxState::Dead,
    }
}

/// Command-hint legend printed below the `ts -l` table. Extracted as a
/// constant so a regression test can assert it never advertises a removed
/// flag (e.g. `--new`) and always lists the live commands.
const LIST_LEGEND: &str =
    "resume: opencode ts -r <id>   delete: opencode ts -d <id>   clean: opencode ts -c";

/// `opencode ts -l` -- global unified panel across every workdir.
///
/// The list is **tmux-first and global**: every live managed tmux session
/// (`opencode-*`, from `tmux list-sessions`) is shown with the tmux pane's
/// actual workdir (`pane_current_path`, `$HOME` abbreviated to `~`), enriched
/// with registry info for the session id. Stopped sessions come from the
/// central ts registry (`<data_root>/ts.db`, one indexed query) — rows are ts
/// sessions by construction — but only once actually started (preview or title
/// present). Never-started empty seeds are not listed.
///
/// Columns: `marker id8 created-ago workdir task-head`.
/// Sorting: non-stopped first, then by workdir path ascending, then by
/// creation time descending (newest first within each path group).
pub(crate) async fn ts_list(_cli: &Cli) -> Result<()> {
    let tmux = list_managed()?;
    let registry = open_registry().await?;
    sync_live_workdirs(&registry, &tmux).await;
    let records = registry.list().await?;
    let mut rows = build_rows(&records, &tmux);
    sort_rows(&mut rows);

    if rows.is_empty() {
        println!("(no sessions)");
        return Ok(());
    }
    for row in &rows {
        let marker = match row.state {
            TmuxState::Attached => "*",
            TmuxState::Detached => "\u{00b7}",
            TmuxState::Dead => "-",
        };
        println!(
            "{marker} {:<10} {:<9} {:<16} {}",
            id8(&row.id),
            format_ts(row.created_at),
            row.path,
            row.task
        );
    }
    println!();
    println!("* attached  \u{00b7} live(detached)  - stopped");
    println!("{}", LIST_LEGEND);
    Ok(())
}

/// One row of the global `ts -l` panel. `path` is the abbreviated durable
/// workdir (or `(unknown)` for legacy rows); `created_at` is epoch milliseconds.
#[derive(Debug, Clone)]
pub(crate) struct GlobalRow {
    pub id: String,
    pub path: String,
    pub created_at: i64,
    pub state: TmuxState,
    pub task: String,
}

/// Merge registry sessions + live managed tmux sessions into the global panel.
///
/// Live tmux sessions always appear, carrying the tmux pane's real workdir;
/// registry rows only *enrich* them (task preview, creation time). A registry
/// session with no live tmux twin is listed as `(stopped)` only when it was
/// actually started (has a preview or title); never-started empty seeds are
/// skipped. Registry rows are ts sessions by construction — the old
/// `model IS NULL` ownership filter lived in this function and is gone.
fn build_rows(records: &[TsRecord], tmux: &[ManagedSession]) -> Vec<GlobalRow> {
    let by_id: HashMap<String, &TsRecord> = records
        .iter()
        .map(|record| (record.id.clone(), record))
        .collect();
    let tmux_by_id: HashMap<String, &ManagedSession> = tmux
        .iter()
        .filter_map(|m| m.id().map(|i| (i.to_string(), m)))
        .collect();

    let mut rows: Vec<GlobalRow> = Vec::with_capacity(tmux.len() + records.len());
    for m in tmux {
        let Some(id) = m.id() else { continue };
        // Prefer the registry's creation time (ms); tmux gives unix seconds.
        let tmux_created_ms = m.created.saturating_mul(1000);
        let (created_at, task) = match by_id.get(id) {
            Some(record) => (record.created_at, task_text(record)),
            None => (tmux_created_ms, "(no task yet)".to_string()),
        };
        rows.push(GlobalRow {
            id: id.to_string(),
            path: abbreviate_path(&m.pane_path),
            created_at,
            state: classify(id, &tmux_by_id),
            task,
        });
    }

    for record in records {
        if tmux_by_id.contains_key(&record.id) {
            continue; // live row already emitted above
        }
        if !has_content(record) {
            continue; // never-started registration-time seed
        }
        rows.push(GlobalRow {
            id: record.id.clone(),
            path: record
                .workdir
                .as_deref()
                .map(|path| abbreviate_path(&path.to_string_lossy()))
                .unwrap_or_else(|| "(unknown)".into()),
            created_at: record.created_at,
            state: TmuxState::Dead,
            task: task_text(record),
        });
    }
    rows
}

/// A stopped registry row counts as a real session only once it was started:
/// the registration-time seed has neither preview nor title.
fn has_content(record: &TsRecord) -> bool {
    !record.preview.trim().is_empty()
        || record
            .title
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
}

/// Task head for a registry session: preview (fallback title) truncated to 20
/// chars, or the `(no task yet)` placeholder when empty.
fn task_text(record: &TsRecord) -> String {
    let raw = preview_of(&record.preview, record.title.as_deref());
    if raw.trim().is_empty() {
        "(no task yet)".to_string()
    } else {
        task_head(raw, 20)
    }
}

/// Sort global rows by workdir ascending, then creation time descending
/// (newest first within each workdir). State does not split a workdir group.
fn sort_rows(rows: &mut [GlobalRow]) {
    rows.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// `opencode ts -r <id>` -- resume/attach a live session, or cold-start a
/// stopped one from its registry record.
pub(crate) async fn ts_resume(cli: &Cli, target: &str) -> Result<()> {
    let tmux = list_managed()?;
    let registry = open_registry().await?;
    let records = registry.list().await?;
    let id = resolve_managed_id(target, &tmux, &records)?;
    if let Some(managed) = tmux
        .iter()
        .find(|managed| managed.id() == Some(id.as_str()))
    {
        return attach(&managed.name);
    }
    // Dead: resolve its durable workdir from the registry.
    let Some(record) = registry.get(&id).await? else {
        bail!(
            "no global tmux session matching `{}`. Run `opencode ts -l` to list.",
            target
        );
    };
    let workdir = match (&cli.workdir, &record.workdir) {
        (Some(explicit), _) => {
            let explicit_dir = opencoder_core::data_dir_for(explicit);
            if record.store_dir.as_deref() != Some(explicit_dir.as_path()) {
                bail!("--workdir does not own global tmux session `{id}`");
            }
            tokio::fs::canonicalize(explicit)
                .await
                .with_context(|| format!("resolve --workdir: {}", explicit.display()))?
        }
        (None, Some(recorded)) => recorded.clone(),
        (None, None) => bail!(
            "global tmux session `{id}` predates workdir tracking; resume once with --workdir <original-path>"
        ),
    };
    register(&registry, &id, &workdir).await?;
    spawn_session(&workdir, &id)
}

/// `opencode ts -c` -- delete stopped ts sessions from every workdir.
pub(crate) async fn ts_cleanup(_cli: &Cli) -> Result<()> {
    let tmux = list_managed()?;
    let registry = open_registry().await?;
    sync_live_workdirs(&registry, &tmux).await;
    let records = registry.list().await?;
    let live_ids: HashSet<&str> = tmux.iter().filter_map(|m| m.id()).collect();
    let targets = cleanup_targets(&records, &live_ids);

    let mut removed = 0u32;
    for (dir, ids) in targets {
        let db = dir.join("opencoder.db");
        let store = LibsqlStore::open(&db)
            .await
            .with_context(|| format!("open store for cleanup: {}", db.display()))?;
        for id in &ids {
            store
                .delete_session(id)
                .await
                .with_context(|| format!("delete stopped ts session {id} from {}", db.display()))?;
            registry.delete(id).await?;
            removed += 1;
        }
    }
    // Rows without an owning store dir have no content to purge; unregister
    // them so cleanup still converges (defensive: no producer creates these).
    for record in &records {
        if record.store_dir.is_none() && !live_ids.contains(record.id.as_str()) {
            registry.delete(&record.id).await?;
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

/// `opencode ts -d <id>` -- remove one exact global managed session.
/// A live tmux instance is terminated first, then its registry record is
/// deleted and its Store content removed. Deleting the caller's current tmux
/// session is refused because killing its pane would interrupt the Store
/// deletion halfway.
pub(crate) async fn ts_delete(target: &str) -> Result<()> {
    let tmux = list_managed()?;
    let registry = open_registry().await?;
    let records = registry.list().await?;
    let id = resolve_managed_id(target, &tmux, &records)?;
    let live = tmux
        .iter()
        .find(|managed| managed.id() == Some(id.as_str()));
    let record = records.iter().find(|record| record.id == id);
    if live.is_none() && record.is_none() {
        bail!("no global tmux session matching `{target}`");
    }
    if let Some(managed) = live {
        if current_session_name()?.as_deref() == Some(managed.name.as_str()) {
            bail!("cannot delete the current tmux session; switch to another session first");
        }
        kill_session(&managed.name)?;
    }
    if let Some(record) = record {
        if let Some(dir) = &record.store_dir {
            let db = dir.join("opencoder.db");
            let store = LibsqlStore::open(&db)
                .await
                .with_context(|| format!("open session store for delete: {}", db.display()))?;
            store.delete_session(&record.id).await.with_context(|| {
                format!("delete global tmux session {id} from {}", db.display())
            })?;
        }
        registry.delete(&record.id).await?;
    }
    println!("removed global tmux session {id}");
    Ok(())
}

/// Resolve exactly what `ts -l` displays: its eight-character id prefix, a
/// full bare/prefixed id, or a live tmux `$index`. Prefixes must identify one
/// global id; registry rows are unique per id, so no duplicate handling.
fn resolve_managed_id(
    target: &str,
    tmux: &[ManagedSession],
    records: &[TsRecord],
) -> Result<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        bail!("session id must not be empty");
    }
    if trimmed.starts_with('$') {
        return tmux
            .iter()
            .find(|managed| managed.tmux_id == trimmed)
            .and_then(ManagedSession::id)
            .map(str::to_string)
            .with_context(|| format!("no live managed tmux session matching `{trimmed}`"));
    }
    let normalized = resolve_target(trimmed);
    let query = id_from_name(&normalized).unwrap_or(trimmed);
    if query.is_empty() {
        bail!("session id must not be empty");
    }

    let mut matches = BTreeSet::new();
    matches.extend(
        tmux.iter()
            .filter_map(ManagedSession::id)
            .filter(|id| id.starts_with(query)),
    );
    matches.extend(
        records
            .iter()
            .map(|record| record.id.as_str())
            .filter(|id| id.starts_with(query)),
    );
    if matches.contains(query) {
        return Ok(query.to_string());
    }
    match matches.len() {
        0 => Ok(query.to_string()),
        1 => Ok((*matches.first().expect("one prefix match")).to_string()),
        _ => bail!(
            "ambiguous global tmux session prefix `{target}` matches: {}",
            matches.into_iter().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Pure target selection shared by the cleanup implementation and tests: dead
/// ts rows grouped by the store dir that owns their content.
fn cleanup_targets(
    records: &[TsRecord],
    live_ids: &HashSet<&str>,
) -> BTreeMap<PathBuf, Vec<String>> {
    let mut targets = BTreeMap::<PathBuf, Vec<String>>::new();
    for record in records {
        if live_ids.contains(record.id.as_str()) {
            continue;
        }
        if let Some(dir) = &record.store_dir {
            targets
                .entry(dir.clone())
                .or_default()
                .push(record.id.clone());
        }
    }
    targets
}

/// Steady state: register a live session's pane workdir only when its registry
/// row is missing or has no durable workdir yet, so `ts -l` does not write on
/// every invocation.
async fn sync_live_workdirs(registry: &TsRegistry, tmux: &[ManagedSession]) {
    for managed in tmux {
        let Some(id) = managed.id() else { continue };
        if managed.pane_path.is_empty() {
            continue;
        }
        let needs_register = match registry.get(id).await {
            Ok(Some(record)) => record.workdir.is_none(),
            Ok(None) => true,
            Err(error) => {
                tracing::warn!(session = %managed.name, %error, "ts: cannot read registry for live workdir");
                false
            }
        };
        if needs_register {
            if let Err(error) = register(registry, id, Path::new(&managed.pane_path)).await {
                tracing::warn!(session = %managed.name, %error, "ts: cannot record live workdir");
            }
        }
    }
}

fn current_workdir(cli: &Cli) -> Result<PathBuf> {
    if let Some(w) = &cli.workdir {
        return Ok(w.clone());
    }
    std::env::current_dir().context("get current dir")
}
#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
