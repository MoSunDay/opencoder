//! `opencode ts` actions: start / list / resume / cleanup.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use opencoder_store::{LibsqlStore, SessionListItem, Store};

use crate::Cli;

use super::display::{abbreviate_path, format_ts, id8, ms_to_secs, preview_of, task_head};
use super::env::tmux_available;
use super::naming::{fresh_id, id_from_name, resolve_target, session_name};
use super::registry::{record_workdir, scan_best_effort, scan_required, StoredSession};
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
    if let Some(id) = &cli.session {
        let globally_known = scan_required(&opencoder_core::data_root())
            .await?
            .iter()
            .any(|stored| stored.item.id == *id && is_ts_owned(&stored.item));
        if globally_known {
            return ts_resume(cli, id).await;
        }
    }
    let workdir = current_workdir(cli)?;
    let id = cli.session.clone().unwrap_or_else(fresh_id);
    record_workdir(&workdir).await?;
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

/// Three-way tmux liveness for a Store session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmuxState {
    Attached,
    Detached,
    Dead,
}

/// True when `path` is the stopped-sentinel, so stopped rows sort after live
/// ones regardless of ASCII ordering (`(` would otherwise precede `~`).
/// Classify a Store session's tmux state given the live tmux id map.
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
/// with `/task`-style info from any store that knows the session id. Stopped
/// sessions are taken from **all** per-workdir stores under the data root —
/// not just the current workdir — but only when they were registered by the
/// ts flow (seeded without agent/model) *and* actually started (preview or
/// title present). Plain `tui`/`run` sessions and never-started empty seeds
/// are never listed.
///
/// Columns: `marker id8 created-ago workdir task-head`.
/// Sorting: non-stopped first, then by workdir path ascending, then by
/// creation time descending (newest first within each path group).
pub(crate) async fn ts_list(_cli: &Cli) -> Result<()> {
    let tmux = list_managed()?;
    sync_live_workdirs(&tmux).await;
    let store_items = scan_best_effort(&opencoder_core::data_root()).await;
    let mut rows = build_rows(&store_items, &tmux);
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
            "{} {:<10} {:<9} {:<16} {}",
            marker,
            id8(&row.id),
            format_ts(ms_to_secs(row.created_at)),
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

/// Read every session from every *existing* per-workdir store under the data
/// root. A missing `opencoder.db`, an unreadable entry, or a failing
/// query is skipped with a `tracing::warn` — a display command must never die
/// because of one bad store dir.
/// Merge store sessions + live managed tmux sessions into the global panel.
///
/// Live tmux sessions always appear, carrying the tmux pane's real workdir;
/// store rows only *enrich* them (task preview, creation time). A store
/// session with no live tmux twin is listed as `(stopped)` only when it was
/// registered by the ts flow: its seeded row persists no agent/model (a plain
/// `tui`/`run` session always writes both at creation) **and** it was actually
/// started (has a preview or title). Never-started empty seeds and plain
/// non-tmux sessions are skipped — registration into the panel is explicit.
fn build_rows(store_items: &[StoredSession], tmux: &[ManagedSession]) -> Vec<GlobalRow> {
    let by_id: HashMap<String, &SessionListItem> = store_items
        .iter()
        .map(|stored| (stored.item.id.clone(), &stored.item))
        .collect();
    let tmux_by_id: HashMap<String, &ManagedSession> = tmux
        .iter()
        .filter_map(|m| m.id().map(|i| (i.to_string(), m)))
        .collect();

    let mut rows: Vec<GlobalRow> = Vec::with_capacity(tmux.len() + store_items.len());
    for m in tmux {
        let Some(id) = m.id() else { continue };
        // Prefer the store's creation time (ms); tmux gives unix seconds.
        let tmux_created_ms = m.created.saturating_mul(1000);
        let (created_at, task) = match by_id.get(id) {
            Some(s) => (s.created_at, task_text(s)),
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

    for stored in store_items {
        let s = &stored.item;
        if tmux_by_id.contains_key(&s.id) {
            continue; // live row already emitted above
        }
        if !is_registered(s) {
            continue; // plain tui/run session or never-started seed
        }
        rows.push(GlobalRow {
            id: s.id.clone(),
            path: stored
                .workdir
                .as_deref()
                .map(|path| abbreviate_path(&path.to_string_lossy()))
                .unwrap_or_else(|| "(unknown)".into()),
            created_at: s.created_at,
            state: TmuxState::Dead,
            task: task_text(s),
        });
    }
    rows
}

/// Was this store session registered by the ts flow?
///
/// Registration happens at `ts` seed time, which writes a row with `model`
/// left NULL — a plain `tui`/`run` session always persists the model at first
/// message. A registered row additionally counts only once it was actually
/// started: the empty seed (no preview, no title) is not a session worth
/// listing.
fn is_registered(s: &SessionListItem) -> bool {
    let has_content = !s.preview.trim().is_empty()
        || s.title.as_deref().is_some_and(|t| !t.trim().is_empty());
    is_ts_owned(s) && has_content
}

/// A session row owned by the ts flow. The seed is inserted before the TUI
/// starts with `model` left NULL; plain `tui`/`run` rows always persist the
/// model at first message. The durable ts marker is therefore `model IS NULL`:
/// mode switches (`/act`, `/plan`, `SwitchAgent`) patch only `agent`, so a
/// used ts session keeps `model` NULL even after `persist_agent` set one.
fn is_ts_owned(s: &SessionListItem) -> bool {
    s.model.is_none()
}

/// Task head for a store session: preview (fallback title) truncated to 20
/// chars, or the `(no task yet)` placeholder when empty.
fn task_text(s: &SessionListItem) -> String {
    let raw = preview_of(&s.preview, s.title.as_deref());
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
/// stopped one from Store history.
pub(crate) async fn ts_resume(cli: &Cli, target: &str) -> Result<()> {
    let tmux = list_managed()?;
    let records = scan_required(&opencoder_core::data_root()).await?;
    let id = resolve_managed_id(target, &tmux, &records)?;
    if let Some(managed) = tmux
        .iter()
        .find(|managed| managed.id() == Some(id.as_str()))
    {
        return attach(&managed.name);
    }
    // Dead: resolve its durable workdir from the global registry.
    let matches: Vec<&StoredSession> = records
        .iter()
        .filter(|stored| stored.item.id == id && is_ts_owned(&stored.item))
        .collect();
    if matches.is_empty() {
        bail!(
            "no global tmux session matching `{}`. Run `opencode ts -l` to list.",
            target
        );
    }
    if matches.len() > 1 {
        bail!("ambiguous global tmux session `{id}` exists in multiple stores");
    }
    let stored = matches[0];
    let workdir = match (&cli.workdir, &stored.workdir) {
        (Some(explicit), _) => {
            let explicit_dir = opencoder_core::data_dir_for(explicit);
            if explicit_dir != stored.store_dir {
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
    record_workdir(&workdir).await?;
    spawn_session(&workdir, &id)
}

/// `opencode ts -c` -- delete stopped ts-owned sessions from every workdir
/// store. Live tmux ids are protected globally, including the caller's own
/// attached session. Plain tui/run history is outside this command's scope.
pub(crate) async fn ts_cleanup(_cli: &Cli) -> Result<()> {
    let tmux = list_managed()?;
    sync_live_workdirs(&tmux).await;
    let store_items = scan_required(&opencoder_core::data_root()).await?;
    let live_ids: HashSet<&str> = tmux.iter().filter_map(|m| m.id()).collect();
    let targets = cleanup_targets(&store_items, &live_ids);

    let mut removed = 0u32;
    for (dir, ids) in targets {
        let db = dir.join("opencoder.db");
        let store = LibsqlStore::open(&db)
            .await
            .with_context(|| format!("open store for cleanup: {}", db.display()))?;
        for id in ids {
            store
                .delete_session(&id)
                .await
                .with_context(|| format!("delete stopped ts session {id} from {}", db.display()))?;
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
/// A live tmux instance is terminated first, then its unique ts-owned Store
/// record is deleted. Deleting the caller's current tmux session is refused
/// because killing its pane would interrupt the Store deletion halfway.
pub(crate) async fn ts_delete(target: &str) -> Result<()> {
    let tmux = list_managed()?;
    let records = scan_required(&opencoder_core::data_root()).await?;
    let id = resolve_managed_id(target, &tmux, &records)?;
    let live = tmux
        .iter()
        .find(|managed| managed.id() == Some(id.as_str()));
    let stored: Vec<&StoredSession> = records
        .iter()
        .filter(|record| record.item.id == id && is_ts_owned(&record.item))
        .collect();
    if stored.len() > 1 {
        bail!("ambiguous global tmux session `{id}` exists in multiple stores");
    }
    if live.is_none() && stored.is_empty() {
        bail!("no global tmux session matching `{target}`");
    }
    if let Some(managed) = live {
        if current_session_name()?.as_deref() == Some(managed.name.as_str()) {
            bail!("cannot delete the current tmux session; switch to another session first");
        }
        kill_session(&managed.name)?;
    }
    if let Some(record) = stored.first() {
        let db = record.store_dir.join("opencoder.db");
        let store = LibsqlStore::open(&db)
            .await
            .with_context(|| format!("open session store for delete: {}", db.display()))?;
        store
            .delete_session(&id)
            .await
            .with_context(|| format!("delete global tmux session {id} from {}", db.display()))?;
    }
    println!("removed global tmux session {id}");
    Ok(())
}

/// Resolve exactly what `ts -l` displays: its eight-character id prefix, a
/// full bare/prefixed id, or a live tmux `$index`. Prefixes must identify one
/// global id; duplicate Store rows for that id are checked by the caller.
fn resolve_managed_id(
    target: &str,
    tmux: &[ManagedSession],
    records: &[StoredSession],
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
            .filter(|stored| is_ts_owned(&stored.item))
            .map(|stored| stored.item.id.as_str())
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

/// Pure target selection shared by the cleanup implementation and tests.
fn cleanup_targets(
    store_items: &[StoredSession],
    live_ids: &HashSet<&str>,
) -> BTreeMap<PathBuf, Vec<String>> {
    let mut targets = BTreeMap::<PathBuf, Vec<String>>::new();
    for stored in store_items {
        let session = &stored.item;
        if is_ts_owned(session) && !live_ids.contains(session.id.as_str()) {
            targets
                .entry(stored.store_dir.clone())
                .or_default()
                .push(session.id.clone());
        }
    }
    targets
}

async fn sync_live_workdirs(tmux: &[ManagedSession]) {
    for managed in tmux {
        if managed.pane_path.is_empty() {
            continue;
        }
        if let Err(error) = record_workdir(Path::new(&managed.pane_path)).await {
            tracing::warn!(session = %managed.name, %error, "ts: cannot record live workdir");
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
mod tests {
    use super::*;
    use opencoder_store::SessionMeta;

    #[test]
    fn explicit_attach_target_bare_ts_returns_none() {
        // A bare `ts` (no --session) always creates a new session -- never attaches.
        assert_eq!(explicit_attach_target(None, false), None);
        assert_eq!(explicit_attach_target(None, true), None);
    }

    #[test]
    fn explicit_attach_target_session_exists_attaches() {
        // `--session <id>` whose tmux session is live -> attach to it.
        assert_eq!(
            explicit_attach_target(Some("01ABCD"), true),
            Some(session_name("01ABCD"))
        );
    }

    #[test]
    fn explicit_attach_target_session_not_live_returns_none() {
        // `--session <id>` but the tmux session is dead -> create fresh.
        assert_eq!(explicit_attach_target(Some("01ABCD"), false), None);
    }

    #[test]
    fn bare_ts_always_builds_new_tmux_session_command() {
        let exe = Path::new("/bin/opencoder");
        let workdir = Path::new("/work/repo");
        let outside = spawn_args(exe, workdir, "01ABC", false);
        let inside = spawn_args(exe, workdir, "01ABC", true);
        assert_eq!(outside[0], "new-session");
        assert!(!outside.iter().any(|arg| arg == "-d"));
        assert_eq!(inside[0], "new-session");
        assert!(inside.iter().any(|arg| arg == "-d"));
        assert!(inside.iter().any(|arg| arg == "opencode-01ABC"));
        assert!(inside.iter().any(|arg| arg == "/work/repo"));
    }

    #[test]
    fn managed_target_resolves_list_prefix_full_id_and_tmux_index() {
        let full_id = "01ABCDEFGHJKMNPQRSTVWXYZ12";
        let mut managed = mk_managed(full_id, 0);
        managed.tmux_id = "$7".into();
        let tmux = vec![managed];
        assert_eq!(resolve_managed_id("01ABCDEF", &tmux, &[]).unwrap(), full_id);
        assert_eq!(resolve_managed_id(full_id, &tmux, &[]).unwrap(), full_id);
        assert_eq!(
            resolve_managed_id(&format!("opencode-{full_id}"), &tmux, &[]).unwrap(),
            full_id
        );
        assert_eq!(resolve_managed_id("$7", &tmux, &[]).unwrap(), full_id);
        assert!(resolve_managed_id("$8", &tmux, &[]).is_err());
    }

    #[test]
    fn managed_target_rejects_ambiguous_prefix() {
        let tmux = vec![
            mk_managed("01ABCDEF111111111111111111", 0),
            mk_managed("01ABCDEF222222222222222222", 0),
        ];
        let error = resolve_managed_id("01ABCDEF", &tmux, &[]).unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
    }

    #[test]
    fn managed_target_resolves_stopped_store_prefix() {
        let full_id = "01STORE0ABCDEFGHJKMNPQRSTV";
        let records = vec![mk_stored(
            "/data/store",
            Some("/work/project"),
            mk_item(full_id, None, None, "task", None, 1),
        )];
        assert_eq!(
            resolve_managed_id("01STORE0", &[], &records).unwrap(),
            full_id
        );
    }

    #[test]
    fn list_legend_has_no_removed_flags() {
        // The `--new` flag was removed: a bare `ts` always creates, so the
        // legend must not advertise it. Advertising a dead command is a
        // silent regression this guard catches.
        assert!(
            !LIST_LEGEND.contains("--new"),
            "legend must not reference removed --new: {LIST_LEGEND}"
        );
        assert!(LIST_LEGEND.contains("resume"), "must advertise resume: {LIST_LEGEND}");
        assert!(LIST_LEGEND.contains("clean"), "must advertise clean: {LIST_LEGEND}");
        assert!(LIST_LEGEND.contains("delete"), "must advertise delete: {LIST_LEGEND}");
    }

    fn mk_managed_at(id: &str, attached: u8, pane_path: &str, created: i64) -> ManagedSession {
        ManagedSession {
            name: session_name(id),
            tmux_id: "$0".into(),
            created,
            attached,
            pane_path: pane_path.into(),
        }
    }

    fn mk_managed(id: &str, attached: u8) -> ManagedSession {
        mk_managed_at(id, attached, "/root/proj", 0)
    }

    /// A store `SessionListItem` for build_rows tests. `agent`/`model` mirror
    /// the seeded (None) vs plain-tui/run (Some) distinction.
    fn mk_item(
        id: &str,
        agent: Option<&str>,
        model: Option<&str>,
        preview: &str,
        title: Option<&str>,
        created: i64,
    ) -> SessionListItem {
        SessionListItem {
            id: id.to_string(),
            title: title.map(String::from),
            agent: agent.map(String::from),
            skill: None,
            model: model.map(String::from),
            created_at: created,
            updated_at: created,
            preview: preview.to_string(),
            subagent_running: 0,
            subagent_cancelled: 0,
        }
    }

    fn mk_stored(dir: &str, workdir: Option<&str>, item: SessionListItem) -> StoredSession {
        StoredSession {
            store_dir: PathBuf::from(dir),
            workdir: workdir.map(PathBuf::from),
            item,
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

    fn row(id: &str, path: &str, created: i64, state: TmuxState) -> GlobalRow {
        GlobalRow {
            id: id.to_string(),
            path: path.to_string(),
            created_at: created,
            state,
            task: String::new(),
        }
    }

    #[test]
    fn sort_by_path_then_created_desc() {
        let mut rows = vec![
            row("b", "~/projB", 100, TmuxState::Attached),
            row("a1", "~/projA", 300, TmuxState::Detached),
            row("a2", "~/projA", 400, TmuxState::Dead),
            row("a0", "~/projA", 300, TmuxState::Attached),
        ];
        sort_rows(&mut rows);
        assert_eq!(rows[0].id, "a2");
        assert_eq!(rows[1].id, "a0");
        assert_eq!(rows[2].id, "a1");
        assert_eq!(rows[3].id, "b");
    }

    #[test]
    fn build_rows_unions_global_tmux_and_registered_stopped() {
        let store_items = vec![
            // Registered ts session (seeded: no agent/model), used, currently
            // live in tmux -> enriched live row with the tmux path.
            mk_stored("/data/a", Some("/work/projY"), mk_item("AA1", None, None, "build the api", Some("t"), 200)),
            // Registered ts session, used, tmux dead -> stopped row.
            mk_stored("/data/a", Some("/work/projY"), mk_item("EE5", None, None, "refactor module", None, 150)),
            // Plain tui/run session (agent+model persisted) -> never listed.
            mk_stored("/data/b", Some("/work/projB"), mk_item("BB2", Some("act"), Some("m"), "plain tui", None, 100)),
            // Never-started empty seed (no preview/title) -> never listed.
            mk_stored("/data/c", None, mk_item("CC3", None, None, "", None, 50)),
        ];
        let tmux = vec![
            mk_managed_at("AA1", 1, "/work/projY", 2),
            mk_managed_at("DD1", 0, "/work/projX", 1),
        ];

        let rows = build_rows(&store_items, &tmux);

        assert_eq!(rows.len(), 3, "AA1 live + DD1 live + EE5 stopped; BB2/CC3 excluded");
        let aa = rows.iter().find(|r| r.id == "AA1").expect("live enriched row");
        assert_eq!(aa.state, TmuxState::Attached);
        assert_eq!(aa.path, "/work/projY", "live path comes from tmux pane_current_path");
        assert_eq!(aa.task, "build the api", "task enriched from the store row");
        assert_eq!(aa.created_at, 200, "creation time taken from the store (ms)");
        // Dedupe: AA1 appears exactly once (tmux row wins over a stopped row).
        assert_eq!(rows.iter().filter(|r| r.id == "AA1").count(), 1);

        let dd = rows.iter().find(|r| r.id == "DD1").expect("tmux-only row");
        assert_eq!(dd.state, TmuxState::Detached);
        assert_eq!(dd.path, "/work/projX");
        assert_eq!(dd.task, "(no task yet)", "no store row -> placeholder");
        assert_eq!(dd.created_at, 1000, "tmux created (seconds) converted to ms");

        let ee = rows.iter().find(|r| r.id == "EE5").expect("registered stopped row");
        assert_eq!(ee.state, TmuxState::Dead);
        assert_eq!(ee.path, "/work/projY");
        assert_eq!(ee.task, "refactor module");

        assert!(
            !rows.iter().any(|r| r.id == "BB2"),
            "plain tui session must not appear as stopped"
        );
        assert!(
            !rows.iter().any(|r| r.id == "CC3"),
            "never-started seed must not appear as stopped"
        );
    }

    #[test]
    fn build_rows_skips_never_started_seed_and_unregistered() {
        // Seeded-and-used rows survive; never-started empty seeds and plain
        // tui/run rows (model persisted) are skipped. A mode-switched ts
        // session keeps `model` NULL (only `agent` is patched), so it MUST
        // still count as ts-owned — the regression this guards.
        let store_items = vec![
            mk_stored("/a", Some("/work/a"), mk_item("S1", None, None, "started", Some("t"), 1)),
            mk_stored("/a", Some("/work/a"), mk_item("S2", None, None, "", None, 2)),
            // Used ts session after a mode switch: agent persisted, model still NULL.
            mk_stored("/b", Some("/work/b"), mk_item("S3", Some("act"), None, "switched mode", Some("t"), 3)),
            // Plain tui/run session: model persisted -> never listed.
            mk_stored("/b", Some("/work/b"), mk_item("S4", None, Some("m"), "plain", None, 4)),
        ];
        let rows = build_rows(&store_items, &[]);
        assert_eq!(rows.len(), 2);
        let s1 = rows.iter().find(|r| r.id == "S1").expect("used seed listed");
        assert_eq!(s1.state, TmuxState::Dead);
        assert_eq!(s1.path, "/work/a");
        let s3 = rows.iter().find(|r| r.id == "S3").expect("mode-switched ts row listed");
        assert_eq!(s3.state, TmuxState::Dead);
        assert_eq!(s3.path, "/work/b");
    }

    #[test]
    fn cleanup_targets_are_global_ts_seeds_and_never_live_or_plain() {
        let store_items = vec![
            mk_stored("/store/a", Some("/work/a"), mk_item("DEAD_A", None, None, "used", None, 4)),
            // Empty seeds are ts-owned too and must not leak forever merely
            // because the list intentionally hides them.
            mk_stored("/store/b", Some("/work/b"), mk_item("EMPTY_B", None, None, "", None, 3)),
            mk_stored("/store/c", Some("/work/c"), mk_item("LIVE_C", None, None, "live", None, 2)),
            mk_stored("/store/d", Some("/work/d"), mk_item("PLAIN_D", Some("act"), Some("m"), "plain", None, 1)),
        ];
        let live_ids = HashSet::from(["LIVE_C"]);

        let targets = cleanup_targets(&store_items, &live_ids);

        assert_eq!(
            targets,
            BTreeMap::from([
                (PathBuf::from("/store/a"), vec!["DEAD_A".to_string()]),
                (PathBuf::from("/store/b"), vec!["EMPTY_B".to_string()]),
            ])
        );
    }

    #[tokio::test]
    async fn scan_all_stores_skips_dirs_without_db_and_non_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        // Real store with one session, in <root>/<h1>/opencoder.db.
        let h1 = tmp.path().join("aaaa");
        std::fs::create_dir_all(&h1).unwrap();
        let store = LibsqlStore::open(h1.join("opencoder.db")).await.unwrap();
        let now = opencoder_core::message::now_ms();
        store
            .create_session(&SessionMeta {
                id: "SCAN1".into(),
                title: None,
                agent: None,
                model: None,
                workdir_hash: None,
                created_at: now,
                updated_at: now,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            })
            .await
            .unwrap();
        drop(store);

        // A directory without a db file, and a plain file entry: both skipped.
        std::fs::create_dir_all(tmp.path().join("bbbb")).unwrap();
        std::fs::write(tmp.path().join("not-a-dir"), "x").unwrap();

        let items = scan_best_effort(tmp.path()).await;
        assert_eq!(items.len(), 1, "only the real store contributes sessions");
        assert_eq!(items[0].store_dir, h1, "store dir reported alongside its sessions");
        assert_eq!(items[0].item.id, "SCAN1");

        let required = scan_required(tmp.path()).await.unwrap();
        assert_eq!(required.len(), 1, "strict cleanup scan sees every real store");
        assert_eq!(required[0].store_dir, h1);
        assert_eq!(required[0].item.id, "SCAN1");

        let missing = scan_required(&tmp.path().join("missing"))
            .await
            .unwrap();
        assert!(missing.is_empty(), "a fresh data root has nothing to clean");
    }

    #[test]
    fn now_ms_is_milliseconds() {
        let t = opencoder_core::message::now_ms();
        assert!(t > 1_000_000_000_000, "now_ms should be in milliseconds, got {t}");
    }
}
