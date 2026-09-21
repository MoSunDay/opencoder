//! The node's operator configuration plane: `<data>/operator-config/`.
//!
//! Operator executions must never inherit the interactive TUI/CLI view of
//! the node workdir (shared `opencoder.json` / `.opencoder/config.json`)
//! nor the node daemon user's real `~/.opencoder`. Instead every operator
//! execution is admitted from a dedicated directory below the node data
//! root:
//!
//! - `operator-config/config.json` — the operator-plane base config.
//! - `operator-config/<mcp|cli|skills|ap|schedules>.json` — domain files.
//! - `operator-config/skills/` — operator-owned skill packs (see
//!   `operator_env` materialization).
//!
//! The directory is bootstrapped ONCE from the node's live configuration
//! (files + env as of the first call), then frozen: TUI `/model`, `/config`
//! or daemon env changes can no longer reach operator executions. From
//! there on the plane is maintained only through operator-plane writes —
//! TUI/CLI `Config::save`/`save_global` never touch this directory.
//!
//! `bootstrap` is idempotent and concurrent-safe: every file is published
//! with the same 0600 first-writer-wins contract as the per-execution
//! snapshot, so a lost race keeps the existing plane untouched.

use anyhow::{Context, Result};
use std::path::Path;

/// Node data subdirectory owning the operator configuration plane.
pub(crate) const DIR_NAME: &str = "operator-config";

/// The operator-plane config directory below the node data root.
pub(crate) fn dir(layout: &crate::layout::DirectoryLayout) -> std::path::PathBuf {
    layout.root().join(DIR_NAME)
}

/// Ensure `dir` exists and is bootstrapped from the node's current live
/// configuration. Missing files are published once; existing ones are kept
/// byte-for-byte (the plane outlives any single bootstrap attempt).
pub(crate) fn bootstrap(dir: &std::path::Path, node_workdir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("operator config dir {} creation failed", dir.display()))?;
    let config = dir.join("config.json");
    if !config.exists() {
        let live = opencoder_core::Config::load(node_workdir)?;
        super::operator_env::write_snapshot(&config, &serde_json::to_string_pretty(&live)?)?;
    }
    // Carry the live domain view (mcp/cli/skills/ap/schedules) across once:
    // without this the first operator execution would silently lose the
    // node's MCP servers / skills toggles. Files that do not resolve stay
    // absent — the plane then treats them as "unset", exactly like a
    // project without domain files.
    for key in ["mcp_servers", "cli", "skills", "autopilot", "schedules"] {
        let Some(value) = opencoder_core::Config::effective_domain_value(node_workdir, key) else {
            continue;
        };
        let Some(file) = opencoder_core::Config::domain_file_for(key) else {
            continue;
        };
        let path = dir.join(file);
        if path.exists() {
            continue;
        }
        super::operator_env::write_snapshot(&path, &serde_json::to_string_pretty(&value)?)?;
    }
    Ok(())
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
