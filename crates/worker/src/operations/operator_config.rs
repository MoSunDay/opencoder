//! The node's operator configuration plane: `<data>/operator-config/`.
//!
//! Operator executions must never inherit the interactive TUI/CLI view of
//! the node workdir (shared `opencoder.json` / `.opencoder/config.json`)
//! nor the node daemon user's real `~/.opencoder`. Instead every operator
//! execution is admitted from a dedicated directory below the node data
//! root:
//!
//! - `operator-config/config.json` — the operator-plane base config.
//! - `operator-config/<mcp|cli|skills|schedules>.json` — domain files.
//! - `operator-config/.bootstrap-complete` — private one-shot completion marker.
//! - `operator-config/.bootstrap-lock` — private cross-process attempt lock.
//! - `operator-config/skills/` — operator-owned skill packs (see
//!   `operator_env` materialization).
//!
//! The directory is bootstrapped ONCE from the node's live configuration
//! (files + env as of the first call), then frozen: TUI `/model`, `/config`
//! or daemon env changes can no longer reach operator executions. From
//! there on the plane is maintained only through operator-plane writes —
//! TUI/CLI `Config::save`/`save_global` never touch this directory.
//!
//! `bootstrap` is idempotent and concurrent-safe: a private per-directory
//! file lock serializes attempts, and every output file is published with the
//! same 0600 first-writer-wins contract as the per-execution snapshot, so a
//! lost race keeps the existing plane untouched.

use anyhow::{Context, Result};
use std::path::Path;

/// Node data subdirectory owning the operator configuration plane.
pub(crate) const DIR_NAME: &str = "operator-config";

/// Present only after every live config and domain bootstrap step completed.
/// Its existence, rather than the presence of individual files, freezes the
/// plane: an intentionally absent domain must stay absent on later calls.
const BOOTSTRAP_COMPLETE: &str = ".bootstrap-complete";

/// The operator-plane config directory below the node data root.
pub(crate) fn dir(layout: &crate::layout::DirectoryLayout) -> std::path::PathBuf {
    layout.root().join(DIR_NAME)
}

/// Ensure `dir` exists and bootstraps it once from the node's current live
/// configuration. A private completion marker is published only after the
/// whole first bootstrap succeeds. Once present, bootstrap returns without
/// reading live configuration or filling deliberately absent domain files.
/// Before that marker exists, existing files are kept byte-for-byte and a
/// failed attempt can be retried.
pub(crate) fn bootstrap(dir: &std::path::Path, node_workdir: &std::path::Path) -> Result<()> {
    bootstrap_inner(dir, node_workdir, || {})
}

fn bootstrap_inner(
    dir: &Path,
    node_workdir: &Path,
    after_initial_check: impl FnOnce(),
) -> Result<()> {
    #[cfg(windows)]
    opencoder_core::platform::fs::ensure_private_directory(dir)?;
    #[cfg(not(windows))]
    std::fs::create_dir_all(dir)
        .with_context(|| format!("operator config dir {} creation failed", dir.display()))?;
    let complete = dir.join(BOOTSTRAP_COMPLETE);
    if bootstrap_complete(&complete)? {
        return Ok(());
    }
    after_initial_check();

    // The lock remains held through the final directory sync. A loser that
    // passed the first check must re-check here: another process may have
    // completed the entire bootstrap while this attempt was waiting.
    let lock = open_bootstrap_lock(dir)?;
    fs2::FileExt::lock_exclusive(&lock)
        .with_context(|| format!("operator bootstrap lock in {} failed", dir.display()))?;
    if bootstrap_complete(&complete)? {
        return Ok(());
    }

    let config = dir.join("config.json");
    let missing_domains = ["mcp_servers", "cli", "skills", "schedules"]
        .into_iter()
        .filter_map(|key| {
            let file = opencoder_core::Config::domain_file_for(key)?;
            let path = dir.join(file);
            (!path.exists()).then_some((key, path))
        })
        .collect::<Vec<_>>();
    let missing_config = !config.exists();

    if missing_config || !missing_domains.is_empty() {
        // One Core operation owns the only strict active-Env resolution and
        // returns raw domains from the same reads used to build `live`.
        let keys = missing_domains
            .iter()
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        let (live, values) = opencoder_core::Config::load_with_domain_values(node_workdir, &keys)?;
        if missing_config {
            super::operator_env::write_snapshot(&config, &serde_json::to_string_pretty(&live)?)?;
        }
        for ((_, path), value) in missing_domains.into_iter().zip(values) {
            if let Some(value) = value {
                super::operator_env::write_snapshot(&path, &serde_json::to_string_pretty(&value)?)?;
            }
        }
    }

    // Reuse the snapshot publisher for 0600 creation, first-writer-wins and
    // directory durability. The empty JSON object is inert marker content.
    super::operator_env::write_snapshot(&complete, "{}")
        .context("operator bootstrap completion marker write failed")?;
    Ok(())
}

fn bootstrap_complete(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("operator bootstrap marker {} check failed", path.display())),
    }
}

fn open_bootstrap_lock(dir: &Path) -> Result<std::fs::File> {
    let path = dir.join(".bootstrap-lock");
    crate::migration_io::reject_symlink(&path, "operator bootstrap lock")?;
    let file = match opencoder_core::platform::fs::create_private_file(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::migration_io::reject_symlink(&path, "operator bootstrap lock")?;
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            options
                .open(&path)
                .with_context(|| format!("open operator bootstrap lock {}", path.display()))?
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("create operator bootstrap lock {}", path.display()));
        }
    };
    anyhow::ensure!(
        opencoder_core::platform::fs::private_access(&path)?,
        "operator bootstrap lock must be private: {}",
        path.display()
    );
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(root: &std::path::Path) -> crate::layout::DirectoryLayout {
        crate::layout::DirectoryLayout::new(root.to_path_buf(), None).unwrap()
    }

    fn seeded_workdir(root: &std::path::Path) -> std::path::PathBuf {
        let workdir = root.join("work");
        std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            serde_json::json!({"model": "openai/node-base"}).to_string(),
        )
        .unwrap();
        std::fs::write(
            workdir.join(".opencoder/mcp.json"),
            serde_json::json!({"node-mcp": {"command": "node"}}).to_string(),
        )
        .unwrap();
        workdir
    }

    #[test]
    fn bootstrap_copies_the_live_view_once_and_stays_frozen() {
        let root = tempfile::tempdir().unwrap();
        let workdir = seeded_workdir(root.path());
        let lay = layout(&root.path().join("node"));
        let dir = dir(&lay);

        bootstrap(&dir, &workdir).unwrap();
        assert!(dir.join(BOOTSTRAP_COMPLETE).is_file());
        let cfg = opencoder_core::Config::load_operator(&dir).unwrap();
        assert_eq!(cfg.model, "openai/node-base");
        assert!(
            cfg.mcp_servers.contains_key("node-mcp"),
            "domain files bootstrap from the live view: {:?}",
            cfg.mcp_servers
        );
        // Files are private (they carry api keys).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "operator config must be owner-only");
        }

        // The TUI side rewrites the shared workdir config afterwards: the
        // operator plane must not follow.
        std::fs::write(
            workdir.join("opencoder.json"),
            serde_json::json!({"model": "openai/tui-changed"}).to_string(),
        )
        .unwrap();
        bootstrap(&dir, &workdir).unwrap();
        let cfg = opencoder_core::Config::load_operator(&dir).unwrap();
        assert_eq!(
            cfg.model, "openai/node-base",
            "operator plane is frozen after bootstrap"
        );
    }

    #[test]
    fn completed_bootstrap_never_backfills_domains_after_an_env_switch() {
        let root = tempfile::tempdir().unwrap();
        let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
        let workdir = root.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            serde_json::json!({"model": "node/original"}).to_string(),
        )
        .unwrap();
        let lay = layout(&root.path().join("node"));
        let dir = dir(&lay);

        // The first complete view intentionally has no CLI or Skill domain.
        bootstrap(&dir, &workdir).unwrap();
        let complete = dir.join(BOOTSTRAP_COMPLETE);
        assert!(complete.is_file());
        assert!(opencoder_core::platform::fs::private_access(&complete).unwrap());
        assert!(!dir.join("cli.json").exists());
        assert!(!dir.join("skills.json").exists());

        let env = opencoder_core::create_config_env("later").unwrap();
        std::fs::write(
            env.path.join("cli.json"),
            serde_json::json!({"later-cli": {"enabled": true, "content": "later"}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            env.path.join("skills.json"),
            serde_json::json!({"later-skill": {"enabled": true}}).to_string(),
        )
        .unwrap();
        opencoder_core::activate_config_env("later").unwrap();

        // A malformed live file also proves the completed path performs no
        // hidden Config::load before returning.
        std::fs::write(workdir.join("opencoder.json"), "not json").unwrap();
        bootstrap(&dir, &workdir).unwrap();
        assert!(!dir.join("cli.json").exists());
        assert!(!dir.join("skills.json").exists());
        let cfg = opencoder_core::Config::load_operator(&dir).unwrap();
        assert_eq!(cfg.model, "node/original");
        assert!(cfg.cli.is_empty());
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn failed_first_bootstrap_has_no_completion_marker_and_can_retry() {
        let root = tempfile::tempdir().unwrap();
        let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
        let workdir = root.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("opencoder.json"), "not json").unwrap();
        let lay = layout(&root.path().join("node"));
        let dir = dir(&lay);
        let complete = dir.join(BOOTSTRAP_COMPLETE);

        assert!(bootstrap(&dir, &workdir).is_err());
        assert!(!complete.exists(), "a failed attempt must stay retryable");
        assert!(!dir.join("config.json").exists());

        std::fs::write(
            workdir.join("opencoder.json"),
            serde_json::json!({"model": "node/retry"}).to_string(),
        )
        .unwrap();
        bootstrap(&dir, &workdir).unwrap();
        assert!(complete.is_file());
        assert_eq!(
            opencoder_core::Config::load_operator(&dir).unwrap().model,
            "node/retry"
        );
    }

    #[test]
    fn partial_bootstrap_retry_propagates_marker_errors_before_legacy_backfill() {
        let root = tempfile::tempdir().unwrap();
        let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
        let workdir = root.path().join("work");
        std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
        std::fs::write(
            workdir.join("opencoder.json"),
            serde_json::json!({"model": "node/retry"}).to_string(),
        )
        .unwrap();
        for (file, value) in [
            (
                "mcp.json",
                serde_json::json!({"legacy-mcp": {"command": "mcp"}}),
            ),
            ("schedules.json", serde_json::json!({"schedules": []})),
        ] {
            std::fs::write(workdir.join(".opencoder").join(file), value.to_string()).unwrap();
        }
        let dir = root.path().join("operator");
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json");
        let original = br#"{"model":"operator/preserved"}"#;
        std::fs::write(&config, original).unwrap();
        let marker = opencoder_core::config_env_status().unwrap().marker_path;
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::create_dir(&marker).unwrap();

        assert!(bootstrap(&dir, &workdir).is_err());
        assert!(!dir.join(BOOTSTRAP_COMPLETE).exists());
        assert_eq!(std::fs::read(&config).unwrap(), original);
        for file in ["mcp.json", "schedules.json", "cli.json", "skills.json"] {
            assert!(!dir.join(file).exists(), "unexpected backfill: {file}");
        }

        std::fs::remove_dir(&marker).unwrap();
        bootstrap(&dir, &workdir).unwrap();
        assert!(dir.join(BOOTSTRAP_COMPLETE).is_file());
        assert_eq!(std::fs::read(&config).unwrap(), original);
        let cfg = opencoder_core::Config::load_operator(&dir).unwrap();
        assert_eq!(cfg.model, "operator/preserved");
        assert!(cfg.mcp_servers.contains_key("legacy-mcp"));
        assert!(dir.join("schedules.json").is_file());
    }

    #[test]
    fn waiting_loser_rechecks_completion_without_reading_or_backfilling() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("operator");
        let workdir = root.path().join("work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        // Any live read by the loser is observable as an error.
        std::fs::write(workdir.join("opencoder.json"), "not json").unwrap();
        let config = dir.join("config.json");
        let mcp = dir.join("mcp.json");
        std::fs::write(&config, br#"{"model":"operator/existing"}"#).unwrap();
        std::fs::write(&mcp, br#"{"existing-mcp":{"command":"existing"}}"#).unwrap();
        let original_config = std::fs::read(&config).unwrap();
        let original_mcp = std::fs::read(&mcp).unwrap();
        let winner_lock = open_bootstrap_lock(&dir).unwrap();
        fs2::FileExt::lock_exclusive(&winner_lock).unwrap();
        let (checked_tx, checked_rx) = std::sync::mpsc::channel();
        let loser_dir = dir.clone();
        let loser_workdir = workdir.clone();
        let loser = std::thread::spawn(move || {
            bootstrap_inner(&loser_dir, &loser_workdir, || {
                checked_tx.send(()).unwrap();
            })
        });

        // The loser has passed its initial missing-marker check. Complete the
        // winner while still owning the lock, then let the loser continue.
        checked_rx.recv().unwrap();
        super::super::operator_env::write_snapshot(&dir.join(BOOTSTRAP_COMPLETE), "{}").unwrap();
        fs2::FileExt::unlock(&winner_lock).unwrap();
        loser.join().unwrap().unwrap();

        assert_eq!(std::fs::read(&config).unwrap(), original_config);
        assert_eq!(std::fs::read(&mcp).unwrap(), original_mcp);
        for file in ["cli.json", "skills.json", "schedules.json"] {
            assert!(!dir.join(file).exists(), "loser backfilled {file}");
        }
        assert!(
            opencoder_core::platform::fs::private_access(&dir.join(".bootstrap-lock")).unwrap()
        );
    }

    #[test]
    fn bootstrap_is_idempotent_and_safe_on_a_live_plane() {
        let root = tempfile::tempdir().unwrap();
        let workdir = root.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        let lay = layout(&root.path().join("node"));
        let dir = dir(&lay);
        bootstrap(&dir, &workdir).unwrap();
        // Operator-plane maintenance edits its own file, then a second
        // bootstrap must not clobber it with the (still stale) node view.
        std::fs::write(
            dir.join("config.json"),
            serde_json::json!({"model": "operator/maintained"}).to_string(),
        )
        .unwrap();
        bootstrap(&dir, &workdir).unwrap();
        let cfg = opencoder_core::Config::load_operator(&dir).unwrap();
        assert_eq!(cfg.model, "operator/maintained");
    }
}

/// Freeze the operator skill pool into the execution home:
/// `<data>/operator-config/skills/` packs plus this binary's embedded
/// built-ins, written ONCE into `<home>/.opencoder/skills` (update-on-drift
/// for built-ins, first-writer-wins for operator packs). The interactive
/// user's global pool is never a source.
pub(crate) fn freeze_skills(layout: &crate::layout::DirectoryLayout, home: &Path) -> Result<()> {
    let target = home.join(".opencoder").join("skills");
    std::fs::create_dir_all(&target)
        .with_context(|| format!("execution skills dir {} creation failed", target.display()))?;
    let source = dir(layout).join("skills");
    if source.is_dir() {
        copy_skill_packs(&source, &target)?;
    }
    opencoder_core::seed_builtin_skills_in(&target)
        .with_context(|| format!("builtin skill seed into {}", target.display()))?;
    opencoder_core::seed_dep_gated_skills_in(&target)
        .with_context(|| format!("dep-gated skill seed into {}", target.display()))?;
    Ok(())
}

/// Copy an operator skill pack tree into the execution home. Accepts both
/// on-disk layouts (`<name>.md`, `<name>/SKILL.md`); directories are copied
/// recursively with symlink cycles rejected.
fn copy_skill_packs(source: &Path, target: &Path) -> Result<()> {
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("operator skill pool {} unreadable", source.display()))?
    {
        let entry = entry.with_context(|| format!("read {}", source.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let dest = target.join(&name);
        if path.is_dir() {
            std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
            copy_tree(&path, &dest)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") && !dest.exists() {
            std::fs::copy(&path, &dest)
                .with_context(|| format!("copy {} -> {}", path.display(), dest.display()))?;
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let canonical = source.canonicalize().context("canonicalize skill source")?;
    anyhow::ensure!(
        !target.starts_with(&canonical),
        "skill source {} contains its copy target",
        source.display()
    );
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let dest = target.join(entry.file_name());
        if path.is_dir() {
            std::fs::create_dir_all(&dest)?;
            copy_tree(&path, &dest)?;
        } else if !dest.exists() {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod freeze_tests {
    use super::*;
    use crate::layout::DirectoryLayout;

    #[test]
    fn freeze_seeds_operator_packs_and_builtins_not_the_user_pool() {
        let root = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(root.path().join("node"), None).unwrap();
        let home = root.path().join("execution-home");
        std::fs::create_dir_all(&home).unwrap();

        // Operator-plane skill pool: one flat .md pack + one directory pack.
        let pool = dir(&layout).join("skills");
        std::fs::create_dir_all(&pool).unwrap();
        std::fs::write(pool.join("runbook.md"), "operator runbook").unwrap();
        std::fs::create_dir_all(pool.join("dir-pack")).unwrap();
        std::fs::write(pool.join("dir-pack").join("SKILL.md"), "operator dir pack").unwrap();

        // A decoy interactive-user pool on disk: freezing must never read it.
        let user_pool = root
            .path()
            .join("user-home")
            .join(".opencoder")
            .join("skills");
        std::fs::create_dir_all(&user_pool).unwrap();
        std::fs::write(user_pool.join("user-private.md"), "interactive user pack").unwrap();

        freeze_skills(&layout, &home).unwrap();
        let target = home.join(".opencoder").join("skills");
        assert!(
            target.join("runbook.md").is_file(),
            "operator md pack frozen into the execution home"
        );
        assert!(
            target.join("dir-pack").join("SKILL.md").is_file(),
            "operator directory pack frozen into the execution home"
        );
        assert!(
            !target.join("user-private.md").exists(),
            "the interactive user's global pool must not leak into the execution"
        );
        // Built-in packs shipped with the binary are seeded alongside.
        let count = std::fs::read_dir(&target).unwrap().count();
        assert!(count > 2, "builtin packs seeded, got {count} entries");
    }
}
