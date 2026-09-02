//! Pure helpers around pure-skill (`$name`) submissions and persisted skill
//! bodies.
//!
//! Skill-only submissions send the raw `$name` text; resolution and the
//! synthetic `SKILL_TRIGGER` injection happen at the runner's consumption
//! boundary (`record_compound` / `entry_drain_mode`), which records the
//! verbatim token as `Message.display` so replay surfaces echo the user's
//! own input — never a resolved trigger body.

/// Derive a display skill name from a persisted body's `> Source:` prefix
/// (`.../skills/<name>/SKILL.md` -> `<name>`). Used to re-sync the TUI's
/// local `active_skill` mirror after the runner activated a skill at
/// consumption time (queue/steer drain): the runner shares only the body
/// through the `skill_prompt` Arc, never the name. For multi-skill joined
/// bodies the first block's name wins (display only — the full body still
/// drives the tail reminder and latent-tool unlocks).
///
/// Flat skill files (`.../skills/<name>.md`) have no per-skill directory: the
/// parent dir is the shared `skills` root, so the name falls back to the file
/// stem (`.../skills/repo.md` -> `repo`).
pub(crate) fn skill_name_from_body(body: &str) -> Option<String> {
    let path = opencoder_session::skill_context::source_path_from_body(body)?;
    let file = std::path::Path::new(path);
    match file.parent().and_then(|dir| dir.file_name()) {
        // Directory-style skill: the owning directory IS the name.
        Some(dir) if dir != "skills" => Some(dir.to_string_lossy().into_owned()),
        // Flat file (parent is the `skills` root, or the path has no parent
        // segment at all): the file stem is the name.
        _ => file.file_stem().map(|s| s.to_string_lossy().into_owned()),
    }
}

/// Backfill the `(active_skill, active_skill_body)` mirrors from a body
/// derived at startup. `run_app` calls this with `initial_skill_state`'s
/// body so a **resumed** skill commit is visible to the loop's mirror reads
/// from the first frame: the idle-submit re-derivation of the `[act]` chip
/// highlight (`resolve_persist` -> `act_plan_highlight`) and the skill-only
/// submit trigger path both read `active_skill`, and the mirror-refresh
/// early-return compares `active_skill_body` against the shared handle.
/// Deriving the name here reuses `skill_name_from_body`, so the mirrors match
/// what a live menu selection would have produced.
pub(crate) fn skill_mirror_from_body(body: Option<String>) -> (Option<String>, Option<String>) {
    let name = body.as_deref().and_then(skill_name_from_body);
    (name, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_derived_from_source_prefix() {
        let body = "> Source: /skills/haiku/SKILL.md\n\nAlways answer in haiku form.";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("haiku"));
    }

    #[test]
    fn multi_skill_body_uses_first_block() {
        let body =
            "> Source: /skills/review/SKILL.md\n\nR\n\n> Source: /skills/submit/SKILL.md\n\nS";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("review"));
    }

    #[test]
    fn body_without_source_prefix_has_no_name() {
        assert_eq!(skill_name_from_body("just instructions"), None);
        assert_eq!(skill_name_from_body(""), None);
    }

    /// Flat skill files (`.../skills/<name>.md`) live directly in the `skills`
    /// root: `parent().file_name()` yields the literal directory name
    /// `skills`, which previously leaked as the display name. The name must
    /// fall back to the file stem.
    #[test]
    fn flat_skill_file_derives_name_from_stem() {
        let body = "> Source: /home/u/.opencoder/skills/task-plan.md\n\nplan body";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("task-plan"));

        // Root-level flat file with no parent segment at all.
        let body = "> Source: flat.md\n\nbody";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("flat"));
    }

    /// The directory-style derivation must be unchanged: a per-skill directory
    /// named anything other than `skills` still wins over the stem.
    #[test]
    fn directory_style_skill_still_uses_parent_dir() {
        let body = "> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nbody";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("task-plan"));
        // A sibling directory merely *called* `skills` would be the root --
        // the stem fallback must also not misfire on a genuinely nested file.
        let body = "> Source: /opt/skills/SKILL.md\n\nbody";
        assert_eq!(skill_name_from_body(body).as_deref(), Some("SKILL"));
    }

    /// The startup mirror backfill pairs the derived name with the body so
    /// `run_app`'s local mirrors match what a live menu selection produced.
    #[test]
    fn mirror_backfill_pairs_derived_name_with_body() {
        let body = "> Source: /skills/task-plan/SKILL.md\n\nbody".to_string();
        let (name, mirrored) = skill_mirror_from_body(Some(body.clone()));
        assert_eq!(name.as_deref(), Some("task-plan"));
        assert_eq!(mirrored.as_deref(), Some(body.as_str()));
        // No body -> both mirrors stay empty.
        assert_eq!(skill_mirror_from_body(None), (None, None));
    }
}
