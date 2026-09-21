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
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::ExecutionKind;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn snapshot_path(home: &Path) -> PathBuf {
    home.join(".opencoder").join("config.json")
}

/// Materialize the per-execution home + workspace for a FRESH operator
/// execution. Idempotent: existing directories are reused and the config
/// snapshot is written once (mirroring `resources::pin` semantics). Returns
/// `None` for every other kind.
pub(crate) fn materialize(
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
    validate_snapshot(&snapshot)?;
    // Freeze the execution's own skill pool: operator-plane packs plus this
    // binary's embedded built-ins. The interactive user's global skill pool
    // (`~/.opencoder/skills`, mirrored by the node-level snapshot) is
    // deliberately NOT a source — an operator execution never sees it.
    crate::operations::operator_config::freeze_skills(layout, &home)?;
    Ok(Some((home, workspace)))
}

/// Atomically create `path` (0600, plain text) without ever replacing a
/// concurrent winner: the first writer wins, later bootstrap attempts keep
/// the existing file. Shared by the per-execution snapshot and the
/// operator-plane bootstrap.
pub(crate) fn write_snapshot(path: &Path, body: &str) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", ulid::Ulid::new()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        validate_snapshot(path)?;
        Ok(())
    })();
    let cleanup = std::fs::remove_file(&temporary);
    result?;
    cleanup?;
    std::fs::File::open(path.parent().context("snapshot parent missing")?)?.sync_all()?;
    Ok(())
}

fn validate_snapshot(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).context("isolated operator config missing")?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o777 == 0o600,
        "isolated operator config must be a private regular file (0600)"
    );
    let _: opencoder_core::Config = serde_json::from_slice(&std::fs::read(path)?)
        .context("isolated operator config is invalid")?;
    Ok(())
}

/// Only explicitly versioned executions use the new environment. A broken
/// isolated execution must never resume in the shared node directory.
pub(crate) fn resolve(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    id: &str,
    version: Option<&serde_json::Value>,
) -> Result<Option<(PathBuf, PathBuf)>> {
    if kind != ExecutionKind::Operator || version.is_none() {
        return Ok(None);
    }
    ensure!(
        version.and_then(serde_json::Value::as_u64) == Some(1),
        "unsupported operator environment version"
    );
    let home = layout.home_dir(kind, id)?;
    let workspace = layout.workspace_dir(kind, id)?;
    ensure!(
        home.is_dir() && workspace.is_dir(),
        "isolated operator directories missing"
    );
    validate_snapshot(&snapshot_path(&home))?;
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
        assert!(resolve(&layout, ExecutionKind::Operator, "op-2", None)
            .unwrap()
            .is_none());
        // Workspace without a snapshot (crash mid-materialize) still falls
        // back only for unversioned historical records.
        opencoder_core::share_fs::durable_create_dir_all(
            &layout
                .workspace_dir(ExecutionKind::Operator, "op-3")
                .unwrap(),
        )
        .unwrap();
        assert!(resolve(&layout, ExecutionKind::Operator, "op-3", None)
            .unwrap()
            .is_none());
        materialize(&layout, ExecutionKind::Operator, "op-2", &config()).unwrap();
        let (home, workspace) = resolve(&layout, ExecutionKind::Operator, "op-2", Some(&json!(1)))
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
        assert!(resolve(&layout, ExecutionKind::Agent, "op-2", None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn versioned_environments_fail_closed_on_missing_or_corrupt_files() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let version = json!(1);
        assert!(resolve(&layout, ExecutionKind::Operator, "missing", Some(&version)).is_err());
        let (home, workspace) =
            materialize(&layout, ExecutionKind::Operator, "op-broken", &config())
                .unwrap()
                .unwrap();
        let snapshot = snapshot_path(&home);
        std::fs::write(&snapshot, "{incomplete").unwrap();
        assert!(resolve(
            &layout,
            ExecutionKind::Operator,
            "op-broken",
            Some(&version)
        )
        .is_err());
        assert!(materialize(&layout, ExecutionKind::Operator, "op-broken", &config()).is_err());
        std::fs::write(&snapshot, serde_json::to_vec(&config()).unwrap()).unwrap();
        std::fs::remove_dir(&workspace).unwrap();
        assert!(resolve(
            &layout,
            ExecutionKind::Operator,
            "op-broken",
            Some(&version)
        )
        .is_err());
        assert!(resolve(&layout, ExecutionKind::Operator, "op-broken", None)
            .unwrap()
            .is_none());
        assert!(resolve(
            &layout,
            ExecutionKind::Operator,
            "op-broken",
            Some(&json!(2))
        )
        .is_err());
    }

    #[test]
    fn concurrent_publishers_only_expose_one_complete_private_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    let mut cfg = config();
                    cfg.model = format!("fixture/model-{i}");
                    write_snapshot(path, &serde_json::to_string(&cfg).unwrap()).unwrap();
                    validate_snapshot(path).unwrap();
                });
            }
        });
        validate_snapshot(&path).unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
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
