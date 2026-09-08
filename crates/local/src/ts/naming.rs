//! Naming contract: managed tmux sessions are `opencoder-<ulid>`, where the
//! ulid is also a real opencoder session id. The legacy `opencode-` prefix
//! (pre-rename) is still *recognized* so live sessions from older binaries
//! keep resolving, but never emitted.

/// Prefix shared by every managed tmux session name.
pub(crate) const TMUX_PREFIX: &str = "opencoder-";

/// Legacy prefix from before the `opencoder` unification; recognized on
/// input only (detect/strip), never used to create new sessions.
pub(crate) const LEGACY_TMUX_PREFIX: &str = "opencode-";

/// Managed tmux session name for an opencoder session id.
pub(crate) fn session_name(id: &str) -> String {
    format!("{TMUX_PREFIX}{id}")
}

/// Inverse of [`session_name`]: the embedded id, or `None` if not managed.
/// Accepts both the current and the legacy prefix.
pub(crate) fn id_from_name(name: &str) -> Option<&str> {
    name.strip_prefix(TMUX_PREFIX)
        .or_else(|| name.strip_prefix(LEGACY_TMUX_PREFIX))
}

/// Fresh opencoder session id (ulid), matching `opencoder_session::runner::new_id`.
pub(crate) fn fresh_id() -> String {
    ulid::Ulid::new().to_string()
}

/// Normalise a user resume target into a concrete tmux target. Accepts
/// `opencoder-<id>` (or the legacy `opencode-<id>`), a bare opencoder ulid
/// (auto-prefixed), or a tmux `$<index>` (unchanged).
pub(crate) fn resolve_target(target: &str) -> String {
    let t = target.trim();
    if t.starts_with('$') || id_from_name(t).is_some() {
        t.to_string()
    } else {
        session_name(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_has_prefix() {
        assert_eq!(session_name("01ABC"), "opencoder-01ABC");
    }

    #[test]
    fn id_from_name_roundtrip() {
        assert_eq!(id_from_name("opencoder-01ABC"), Some("01ABC"));
        assert_eq!(id_from_name("opencoder-"), Some(""));
        assert_eq!(id_from_name("other"), None);
        assert_eq!(id_from_name("opencoderX"), None);
    }

    #[test]
    fn legacy_prefix_is_recognized_but_never_emitted() {
        // Sessions created before the rename keep resolving to their id.
        assert_eq!(id_from_name("opencode-01ABC"), Some("01ABC"));
        assert_eq!(id_from_name("opencode-"), Some(""));
        // But new names always use the unified prefix.
        assert_eq!(session_name("01ABC"), "opencoder-01ABC");
    }

    #[test]
    fn resolve_target_three_forms() {
        assert_eq!(resolve_target("01HZ"), "opencoder-01HZ");
        assert_eq!(resolve_target("opencoder-01HZ"), "opencoder-01HZ");
        // Legacy-prefixed target passes through unchanged (still managed).
        assert_eq!(resolve_target("opencode-01HZ"), "opencode-01HZ");
        assert_eq!(resolve_target("$3"), "$3");
        assert_eq!(resolve_target("  01HZ  "), "opencoder-01HZ");
    }
}
