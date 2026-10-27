//! Per-execution HOME/WORKSPACE isolation for operator executions.
//!
//! An operator execution owns a private `home` and `workspace` below its
//! execution directory:
//!
//! - `home/.opencoder/config.json` — a frozen snapshot of the node `Config`
//!   (written once at creation; node-owned 0600 — on Linux identity nodes
//!   root-owned 0440 after the execution account takes over the trees).
//!   Config resolution for the session
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
//!
//! On Linux each operator execution additionally owns a dedicated system
//! account (`linux_identity`): home/workspace move to it (0700) while the
//! frozen snapshot stays node-owned (0440); the terminal state releases the
//! account and hands the produced files back to the node user.

use crate::layout::DirectoryLayout;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::ExecutionKind;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub mod linux_identity;

mod fork;
pub use fork::fork_environment;

fn snapshot_path(home: &Path) -> PathBuf {
    home.join(".opencoder").join("config.json")
}

/// Materialized operator environment: the private home/workspace plus, on
/// Linux, the provisioned per-execution identity for later privilege drop.
pub struct OperatorEnvironment {
    pub home: PathBuf,
    pub workspace: PathBuf,
    /// Linux-only identity; `None` on other platforms, when provisioning is
    /// disabled, and for resumed environments (the harness runtime carries
    /// it after initialization).
    pub run_as: Option<opencoder_core::platform::RunAsIdentity>,
}

/// Materialize the per-execution home + workspace for a FRESH operator
/// execution. Idempotent: existing directories are reused and the config
/// snapshot is written once (mirroring `resources::pin` semantics). Returns
/// `None` for every other kind.
pub fn materialize(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    id: &str,
    config: &opencoder_core::Config,
) -> Result<Option<OperatorEnvironment>> {
    if kind != ExecutionKind::Operator {
        return Ok(None);
    }
    let (home, workspace) = create_directories(layout, kind, id)?;
    let snapshot = snapshot_path(&home);
    if !snapshot.is_file() {
        write_snapshot(&snapshot, &serde_json::to_string_pretty(config)?)?;
    }
    validate_snapshot(&snapshot)?;
    crate::operations::operator_config::freeze_skills(layout, &home)?;
    // Linux: admission + dedicated account; the private trees move to it
    // and the frozen snapshot is re-owned root:root 0440. Other platforms
    // keep the node-owned layout unchanged.
    #[cfg(target_os = "linux")]
    let run_as = linux_identity::materialize_ownership(config, &home, &workspace, id)?;
    #[cfg(not(target_os = "linux"))]
    let run_as = None;
    Ok(Some(OperatorEnvironment {
        home,
        workspace,
        run_as,
    }))
}

/// Record-annotation payload persisting the provisioned identity
/// (`operator_identity`), so resumes re-attach it to the harness runtime
/// without a live account lookup. `Value::Null` when no identity exists.
pub fn identity_annotation(environment: &Option<OperatorEnvironment>) -> Value {
    environment
        .as_ref()
        .and_then(|env| env.run_as.as_ref())
        .and_then(|run_as| serde_json::to_value(run_as).ok())
        .unwrap_or(Value::Null)
}

/// Terminal-state teardown of the per-execution Linux identity: chown the
/// produced files back to the node user and delete the account. Idempotent
/// (absent account/directories are success); other platforms are no-ops
/// handled by the caller's cfg.
#[cfg(target_os = "linux")]
pub fn release_identity(layout: &DirectoryLayout, kind: ExecutionKind, id: &str) -> Result<()> {
    let home = layout.home_dir(kind, id)?;
    let workspace = layout.workspace_dir(kind, id)?;
    linux_identity::release(&home, &workspace, id)
}

fn create_directories(
    layout: &DirectoryLayout,
    kind: ExecutionKind,
    id: &str,
) -> Result<(PathBuf, PathBuf)> {
    let home = layout.home_dir(kind, id)?;
    let workspace = layout.workspace_dir(kind, id)?;
    #[cfg(not(windows))]
    opencoder_core::share_fs::durable_create_dir_all(&home)?;
    #[cfg(windows)]
    opencoder_core::platform::fs::ensure_private_directory(&home)?;
    opencoder_core::share_fs::durable_create_dir_all(&workspace)?;
    #[cfg(windows)]
    for directory in [home.join("AppData/Roaming"), home.join("AppData/Local")] {
        opencoder_core::platform::fs::ensure_private_directory(&directory)?;
    }
    let config_dir = home.join(".opencoder");
    #[cfg(not(windows))]
    opencoder_core::share_fs::durable_create_dir_all(&config_dir)?;
    #[cfg(windows)]
    opencoder_core::platform::fs::ensure_private_directory(&config_dir)?;
    Ok((home, workspace))
}

/// Atomically create `path` (0600, plain text) without ever replacing a
/// concurrent winner: the first writer wins, later bootstrap attempts keep
/// the existing file. Shared by the per-execution snapshot and the
/// operator-plane bootstrap.
pub(crate) fn write_snapshot(path: &Path, body: &str) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", ulid::Ulid::new()));
    let result = (|| -> Result<()> {
        let mut file = opencoder_core::platform::fs::create_private_file(&temporary)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        match opencoder_core::platform::fs::publish_new(&temporary, path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        validate_snapshot(path)?;
        Ok(())
    })();
    let cleanup = std::fs::remove_file(&temporary);
    result?;
    if let Err(error) = cleanup {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error.into());
        }
    }
    opencoder_core::platform::fs::sync_directory(
        path.parent().context("snapshot parent missing")?,
    )?;
    Ok(())
}

fn validate_snapshot(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).context("isolated operator config missing")?;
    ensure!(
        metadata.is_file() && !opencoder_core::platform::fs::is_link(&metadata),
        "isolated operator config must be a regular file"
    );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.uid() == linux_identity::node_uid(),
            "isolated operator config must stay owned by the node user"
        );
        // 0600: node-only fresh layout (also the legacy pre-identity form).
        // 0440: frozen after the execution account took over the trees.
        let mode = metadata.permissions().mode() & 0o777;
        ensure!(
            mode == 0o600 || mode == 0o440,
            "isolated operator config mode must be 0600 or 0440"
        );
    }
    #[cfg(not(target_os = "linux"))]
    ensure!(
        opencoder_core::platform::fs::private_access(path)?,
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
) -> Result<Option<OperatorEnvironment>> {
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
    Ok(Some(OperatorEnvironment {
        home,
        workspace,
        run_as: None,
    }))
}

/// HOME env pair for harness injection: persisted with the harness runtime
/// so every resume rebuilds `SessionState::env_passthrough` from it.
pub(crate) fn env_pairs(home: &Path) -> Vec<(String, String)> {
    let home = home.to_string_lossy().into_owned();
    let mut pairs = vec![("HOME".into(), home.clone())];
    if cfg!(windows) {
        pairs.extend([
            ("USERPROFILE".into(), home.clone()),
            ("APPDATA".into(), format!("{home}\\AppData\\Roaming")),
            ("LOCALAPPDATA".into(), format!("{home}\\AppData\\Local")),
        ]);
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layout(root: &Path) -> DirectoryLayout {
        DirectoryLayout::new(root.to_path_buf(), None).unwrap()
    }

    fn config() -> opencoder_core::Config {
        // Unit tests keep the node-owned layout; Linux identity provisioning
        // is integration-level (tests/operator_linux_identity.rs).
        opencoder_core::Config {
            model: "openai/gpt-isolation".into(),
            operator: opencoder_core::config::OperatorConfig {
                linux_identity: opencoder_core::config::LinuxIdentityMode::Off,
            },
            ..Default::default()
        }
    }

    fn expect_environment(
        environment: Result<Option<OperatorEnvironment>>,
    ) -> (PathBuf, PathBuf) {
        let environment = environment.unwrap().expect("operator materializes");
        (environment.home, environment.workspace)
    }

    #[test]
    fn materialize_writes_snapshot_once_and_keeps_permissions_private() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let (home, workspace) =
            expect_environment(materialize(&layout, ExecutionKind::Operator, "op-1", &config()));
        assert!(workspace.is_dir());
        let snapshot = snapshot_path(&home);
        let raw = std::fs::read_to_string(&snapshot).unwrap();
        assert!(raw.contains("gpt-isolation"), "snapshot: {raw}");
        assert!(opencoder_core::platform::fs::private_access(&snapshot).unwrap());

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
        let (home, workspace) =
            expect_environment(resolve(&layout, ExecutionKind::Operator, "op-2", Some(&json!(1))));
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
            expect_environment(materialize(&layout, ExecutionKind::Operator, "op-broken", &config()));
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
            pairs[0],
            ("HOME".to_string(), "/node/operator/x/home".to_string())
        );
        assert_eq!(pairs.len(), if cfg!(windows) { 4 } else { 1 });
    }

    /// The snapshot must round-trip as a `Config` so `Config::load_with_home`
    /// reproduces the execution's frozen view (api key included).
    #[test]
    fn snapshot_round_trips_through_config_load_with_home() {
        let root = tempfile::tempdir().unwrap();
        let working = root.path().join("workspace");
        std::fs::create_dir_all(&working).unwrap();
        let layout = layout(root.path());
        let (home, _) =
            expect_environment(materialize(&layout, ExecutionKind::Operator, "op-5", &config()));
        let loaded = opencoder_core::Config::load_with_home(&working, Some(&home)).unwrap();
        assert_eq!(loaded.model, "openai/gpt-isolation");
    }

    #[test]
    fn snapshot_keeps_provider_api_key() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let mut cfg = config();
        cfg.provider.api_key = Some("{E2E_KEY}".into());
        let (home, _) =
            expect_environment(materialize(&layout, ExecutionKind::Operator, "op-6", &cfg));
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(snapshot_path(&home)).unwrap()).unwrap();
        assert_eq!(raw["provider"]["api_key"], json!("{E2E_KEY}"));
    }
}
