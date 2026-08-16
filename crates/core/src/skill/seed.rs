//! Seeding of the built-in and dependency-gated skill packs.
//!
//! Everything that *writes* to `~/.opencoder/skills` on startup lives here:
//! the embedded skill tables ([`BUILTIN_SKILLS`] / [`DEP_GATED_SKILLS`]), the
//! per-file incremental seeders, and the optional-deps install script. The
//! read side of the module (discovery, parsing, skill tokens) stays in
//! [`crate::skill`].

use std::path::Path;

use super::skills_dir;

/// Built-in skills shipped with the binary and embedded at compile time via
/// [`include_str!`]. Each entry is `(skill_dir, &[(file_name, contents)])`.
/// Seeded into `~/.opencoder/skills` on first startup so a fresh install ships
/// the `task-plan -> do-and-done -> review -> submit` workflow, the orthogonal
/// `summary` retrospective tool (read-only task recap at any checkpoint), and
/// the `say-and-replay` alignment snapshot tool (read-only progress replay),
/// plus the memory pair `repo-local-memory` (per-iteration repair-on-touch
/// minimal updates) and `repo-local-dreaming` (periodic full memory
/// consolidation).
const BUILTIN_SKILLS: &[(&str, &[(&str, &str)])] = &[
    (
        "task-plan",
        &[("SKILL.md", include_str!("../../assets/skills/task-plan/SKILL.md"))],
    ),
    (
        "do-and-done",
        &[(
            "SKILL.md",
            include_str!("../../assets/skills/do-and-done/SKILL.md"),
        )],
    ),
    (
        "repo-local-memory",
        &[
            (
                "SKILL.md",
                include_str!("../../assets/skills/repo-local-memory/SKILL.md"),
            ),
            (
                "EXAMPLES.md",
                include_str!("../../assets/skills/repo-local-memory/EXAMPLES.md"),
            ),
            (
                "TEMPLATES.md",
                include_str!("../../assets/skills/repo-local-memory/TEMPLATES.md"),
            ),
        ],
    ),
    (
        "repo-local-dreaming",
        &[("SKILL.md", include_str!("../../assets/skills/repo-local-dreaming/SKILL.md"))],
    ),
    (
        "say-and-replay",
        &[("SKILL.md", include_str!("../../assets/skills/say-and-replay/SKILL.md"))],
    ),
    (
        "review",
        &[("SKILL.md", include_str!("../../assets/skills/review/SKILL.md"))],
    ),
    (
        "summary",
        &[("SKILL.md", include_str!("../../assets/skills/summary/SKILL.md"))],
    ),
    (
        "submit",
        &[("SKILL.md", include_str!("../../assets/skills/submit/SKILL.md"))],
    ),
];

/// Dependency-gated skills - hidden until the user runs
/// `install-skills-dep.sh` which creates a sentinel file in `skills_dir()`.
/// Seeded independently of [`BUILTIN_SKILLS`] so a fresh install does not get
/// these skills unless the user opted in. ssh-pty needs tmux; chrome-headless
/// needs a locally installed Chrome/Chromium (not bundled - the skill detects it).
const DEP_GATED_SKILLS: &[(&str, &[(&str, &str)])] = &[
    (
        "ssh-pty",
        &[(
            "SKILL.md",
            include_str!("../../assets/skills/ssh-pty/SKILL.md"),
        )],
    ),
    (
        "chrome-headless",
        &[(
            "SKILL.md",
            include_str!("../../assets/skills/chrome-headless/SKILL.md"),
        )],
    ),
];

/// Sentinel file (inside [`crate::skill::skills_dir`]) whose presence means the
/// user ran `install-skills-dep.sh` and the optional-dependency skills should
/// be seeded. Independent of built-in skill seeding.
pub const DEPS_SENTINEL: &str = ".skills-deps";

/// Seed the built-in skills into `~/.opencoder/skills`.
///
/// Incremental and best-effort: every shipped skill is checked individually —
/// missing files are written, existing files are never clobbered (so user edits
/// survive). This means a binary upgrade that ships a *new* built-in skill
/// lands it on the next startup even for users who installed an earlier
/// version. Errors are logged via `tracing` and never propagated — seeding
/// must never block startup.
pub fn seed_builtin_skills() {
    let root = skills_dir();
    if let Err(e) = seed_builtin_skills_in(&root) {
        tracing::warn!(
            "failed to seed built-in skills into {}: {e}",
            root.display()
        );
    }
}

/// Filesystem-writing core, factored out so tests can target a tempdir.
///
/// Incremental: creates each skill directory and writes files that don't yet
/// exist, never overwriting existing files (user edits survive). This is the
/// single source of seeding logic — the public [`seed_builtin_skills`] entry
/// point merely resolves `~/.opencoder/skills` and forwards here.
pub fn seed_builtin_skills_in(root: &Path) -> std::io::Result<()> {
    for (skill_dir, files) in BUILTIN_SKILLS {
        let dir = root.join(skill_dir);
        std::fs::create_dir_all(&dir)?;
        for (name, content) in *files {
            let path = dir.join(name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Seed the dependency-gated skills (ssh-pty) into
/// `~/.opencoder/skills` if the [`DEPS_SENTINEL`] file exists.
///
/// Independent of [`seed_builtin_skills`]: a fresh install gets only the
/// built-in skills until the user explicitly installs the optional deps via
/// `install-skills-dep.sh`. Idempotent and best-effort.
pub fn seed_dep_gated_skills() {
    let root = skills_dir();
    if !root.join(DEPS_SENTINEL).exists() {
        return;
    }
    if let Err(e) = seed_dep_gated_skills_in(&root) {
        tracing::warn!(
            "failed to seed dep-gated skills into {}: {e}",
            root.display()
        );
    }
}

/// Filesystem-writing core for dep-gated skills, factored out for tests.
/// Like [`seed_builtin_skills_in`] but writes the dep-gated set; never
/// overwrites existing files. Sentinel-gated: writes nothing unless
/// [`DEPS_SENTINEL`] exists under `root`, mirroring the gate in
/// [`seed_dep_gated_skills`] so the contract is testable against a tempdir.
pub fn seed_dep_gated_skills_in(root: &Path) -> std::io::Result<()> {
    if !root.join(DEPS_SENTINEL).exists() {
        return Ok(());
    }
    for (skill_dir, files) in DEP_GATED_SKILLS {
        let dir = root.join(skill_dir);
        std::fs::create_dir_all(&dir)?;
        for (name, content) in *files {
            let path = dir.join(name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Write `install-skills-dep.sh` into `~/.opencoder/` so the user can discover
/// and run it. Idempotent: skips if the file already exists.
pub fn write_install_script() {
    let dir = match dirs::home_dir() {
        Some(h) => h.join(".opencoder"),
        None => return,
    };
    if let Err(e) = write_install_script_in(&dir) {
        tracing::warn!("failed to write install script to {}: {e}", dir.display());
    }
}

/// Filesystem-writing core for the install script, factored out so tests can
/// target a tempdir. Idempotent: skips if the file already exists. Sets
/// executable permissions on Unix.
pub fn write_install_script_in(base: &Path) -> std::io::Result<()> {
    let path = base.join("install-skills-dep.sh");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, INSTALL_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, PermissionsExt::from_mode(0o755))?;
    }
    Ok(())
}

/// Embedded copy of `scripts/install-skills-dep.sh`, written to
/// `~/.opencoder/install-skills-dep.sh` on startup so users can discover the
/// optional-dependency installer.
const INSTALL_SCRIPT: &str = include_str!("../../../../scripts/install-skills-dep.sh");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_install_script_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_install_script_in(base).unwrap();
        let script = base.join("install-skills-dep.sh");
        assert!(script.is_file());
        let content = std::fs::read_to_string(&script).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn write_install_script_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_install_script_in(base).unwrap();
        // Write a sentinel to detect overwrite.
        let script = base.join("install-skills-dep.sh");
        std::fs::write(&script, "SENTINEL").unwrap();
        write_install_script_in(base).unwrap();
        let content = std::fs::read_to_string(&script).unwrap();
        assert_eq!(content, "SENTINEL");
    }
}
