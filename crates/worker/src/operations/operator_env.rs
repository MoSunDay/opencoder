//! Per-execution HOME/WORKSPACE isolation for operator executions.
//!
//! An operator execution owns a private `home` and `workspace` below its
//! execution directory:
//!
//! - `home/.opencoder/config.json` — a frozen snapshot of the node `Config`
//!   (written once at creation, 0600). Config resolution for the session
//!   (`Config::load_with_home`) redirects the global candidates here, so the
//!   session never pierces the node daemon's — i.e. the interactive TUI
//!   user's — `~/.opencoder`.
//! - `workspace` — the session working directory (bash cwd), decoupled from
//!   the node-level workdir shared with TUI-style sessions.
//!
//! `HOME` itself reaches tool subprocesses through the harness env pairs
//! (the same durable `env_passthrough` mechanism as `OPENCODER_HOW_APPEND`),
//! so a resumed session rebuilds it after a node restart.
//!
//! Scope: `ExecutionKind::Operator` only. Maintenance stays node-level (its
//! job is node maintenance) and agent-kind host/runc loops are untouched.
//! Legacy in-flight executions (created before this layout existed) resolve
//! to `None` and keep their node-level behavior.

use crate::layout::DirectoryLayout;
use anyhow::Result;
use opencoder_core::fleet::ExecutionKind;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Snapshot file that marks an execution as isolated: `resolve` returns
/// `Some` only when it exists, so a crash between directory creation and
/// the snapshot write falls back to the legacy node-level paths instead of
/// stranding a session on a HOME with no config.
fn snapshot_path(home: &Path) -> PathBuf {
    home.join(".opencoder").join("config.json")
}

/// Materialize the per-execution home + workspace for a FRESH operator
/// execution. Idempotent: existing directories are reused and the config
/// snapshot is written once (mirroring `resources::pin` semantics). Returns
/// `None` for every other kind.
pub(super) fn materialize(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    id: &str,
    config: &opencoder_core::Config,
) -> Result<Option<(PathBuf, PathBuf)>> {
    if kind != ExecutionKind::Operator {
        return Ok(None);
    }
    let home = layout.home_dir(kind, id)?;
    let workspace = layout.workspace_dir(kind, id)?;
    opencoder_core::share_fs::durable_create_dir_all(&home)?;
    opencoder_core::share_fs::durable_create_dir_all(&workspace)?;
    let snapshot = snapshot_path(&home);
    if !snapshot.is_file() {
        let parent = snapshot
            .parent()
            .ok_or_else(|| anyhow::anyhow!("snapshot path has no parent"))?;
        opencoder_core::share_fs::durable_create_dir_all(parent)?;
        write_snapshot(&snapshot, &serde_json::to_string_pretty(config)?)?;
    }
    Ok(Some((home, workspace)))
}

/// Write the config snapshot 0600, durably. `create_new` keeps the
/// once-only guarantee under concurrent callers; an `AlreadyExists` race is
/// treated as success (the winner's snapshot is authoritative).
fn write_snapshot(path: &Path, body: &str) -> Result<()> {
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    file.write_all(body.as_bytes())?;
    file.sync_all()?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Resolve the materialized per-execution env for an existing execution.
/// `Some((home, workspace))` only when the snapshot exists (i.e. the
/// execution was created after the isolation change and materialized
/// successfully); legacy records resolve to `None` so resumed behavior is
/// unchanged.
pub(crate) fn resolve(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    id: &str,
) -> Result<Option<(PathBuf, PathBuf)>> {
    if kind != ExecutionKind::Operator {
        return Ok(None);
    }
    let home = layout.home_dir(kind, id)?;
    let workspace = layout.workspace_dir(kind, id)?;
    if !snapshot_path(&home).is_file() || !workspace.is_dir() {
        return Ok(None);
    }
    Ok(Some((home, workspace)))
}

/// HOME env pair for harness injection: persisted with the harness runtime
/// so every resume rebuilds `SessionState::env_passthrough` from it.
pub(crate) fn env_pairs(home: &Path) -> Vec<(String, String)> {
    vec![("HOME".to_string(), home.to_string_lossy().into_owned())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layout(root: &Path) -> DirectoryLayout {
        DirectoryLayout::new(root.to_path_buf(), None).unwrap()
    }

    fn config() -> opencoder_core::Config {
        opencoder_core::Config {
            model: "openai/gpt-isolation".into(),
            ..Default::default()
        }
    }

    #[test]
    fn materialize_writes_snapshot_once_and_keeps_permissions_private() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let (home, workspace) = materialize(&layout, ExecutionKind::Operator, "op-1", &config())
            .unwrap()
            .expect("operator materializes");
        assert!(workspace.is_dir());
        let snapshot = snapshot_path(&home);
        let raw = std::fs::read_to_string(&snapshot).unwrap();
        assert!(raw.contains("gpt-isolation"), "snapshot: {raw}");
        let mode = std::fs::metadata(&snapshot).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "snapshot must be private");

        // Idempotent: a second materialize with a DIFFERENT config must not
        // rewrite the frozen snapshot.
        let mut changed = config();
        changed.model = "openai/gpt-rotated".into();
        materialize(&layout, ExecutionKind::Operator, "op-1", &changed).unwrap();
        assert!(
            !std::fs::read_to_string(&snapshot)
                .unwrap()
                .contains("gpt-rotated"),
            "snapshot is frozen after first write"
        );
    }

    #[test]
    fn resolve_requires_snapshot_and_is_operator_only() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        // No materialize yet: legacy record falls back.
        assert!(resolve(&layout, ExecutionKind::Operator, "op-2")
            .unwrap()
            .is_none());
        // Workspace without a snapshot (crash mid-materialize) still falls
        // back — HOME without a config would strand the session.
        opencoder_core::share_fs::durable_create_dir_all(
            &layout
                .workspace_dir(ExecutionKind::Operator, "op-3")
                .unwrap(),
        )
        .unwrap();
        assert!(resolve(&layout, ExecutionKind::Operator, "op-3")
            .unwrap()
            .is_none());
        materialize(&layout, ExecutionKind::Operator, "op-2", &config()).unwrap();
        let (home, workspace) = resolve(&layout, ExecutionKind::Operator, "op-2")
            .unwrap()
            .expect("materialized operator resolves");
        assert_eq!(
            workspace,
            layout
                .workspace_dir(ExecutionKind::Operator, "op-2")
                .unwrap()
        );
        assert_eq!(
            home,
            layout.home_dir(ExecutionKind::Operator, "op-2").unwrap()
        );

        // Other kinds never materialize nor resolve.
        assert!(
            materialize(&layout, ExecutionKind::Maintenance, "op-4", &config())
                .unwrap()
                .is_none()
        );
        assert!(
            materialize(&layout, ExecutionKind::Agent, "op-4", &config())
                .unwrap()
                .is_none()
        );
        assert!(resolve(&layout, ExecutionKind::Agent, "op-2")
            .unwrap()
            .is_none());
    }

    #[test]
    fn env_pairs_point_home_at_the_execution_home() {
        let pairs = env_pairs(Path::new("/node/operator/x/home"));
        assert_eq!(
            pairs,
            vec![("HOME".to_string(), "/node/operator/x/home".to_string())]
        );
    }

    /// The snapshot must round-trip as a `Config` so `Config::load_with_home`
    /// reproduces the execution's frozen view (api key included).
    #[test]
    fn snapshot_round_trips_through_config_load_with_home() {
        let root = tempfile::tempdir().unwrap();
        let working = root.path().join("workspace");
        std::fs::create_dir_all(&working).unwrap();
        let layout = layout(root.path());
        let (home, _) = materialize(&layout, ExecutionKind::Operator, "op-5", &config())
            .unwrap()
            .unwrap();
        let loaded = opencoder_core::Config::load_with_home(&working, Some(&home)).unwrap();
        assert_eq!(loaded.model, "openai/gpt-isolation");
    }

    #[test]
    fn snapshot_keeps_provider_api_key() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let mut cfg = config();
        cfg.provider.api_key = Some("{E2E_KEY}".into());
        let (home, _) = materialize(&layout, ExecutionKind::Operator, "op-6", &cfg)
            .unwrap()
            .unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(snapshot_path(&home)).unwrap()).unwrap();
        assert_eq!(raw["provider"]["api_key"], json!("{E2E_KEY}"));
    }
}
