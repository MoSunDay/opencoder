//! Naming contract: managed tmux sessions are `opencode-<ulid>`, where the
//! ulid is also a real opencode session id.

/// Prefix shared by every managed tmux session name.
pub(crate) const TMUX_PREFIX: &str = "opencode-";

/// Managed tmux session name for an opencode session id.
pub(crate) fn session_name(id: &str) -> String {
    format!("{TMUX_PREFIX}{id}")
}

/// Inverse of [`session_name`]: the embedded id, or `None` if not managed.
pub(crate) fn id_from_name(name: &str) -> Option<&str> {
    name.strip_prefix(TMUX_PREFIX)
}

/// Fresh opencode session id (ulid), matching `opencoder_session::runner::new_id`.
pub(crate) fn fresh_id() -> String {
    ulid::Ulid::new().to_string()
}

/// Normalise a user resume target into a concrete tmux target. Accepts
/// `opencode-<id>`, a bare opencode ulid (auto-prefixed), or a tmux `$<index>`
/// (unchanged).
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
        assert_eq!(session_name("01ABC"), "opencode-01ABC");
    }

    #[test]
    fn id_from_name_roundtrip() {
        assert_eq!(id_from_name("opencode-01ABC"), Some("01ABC"));
        assert_eq!(id_from_name("opencode-"), Some(""));
        assert_eq!(id_from_name("other"), None);
        assert_eq!(id_from_name("opencodeX"), None);
    }

    #[test]
    fn resolve_target_three_forms() {
        assert_eq!(resolve_target("01HZ"), "opencode-01HZ");
        assert_eq!(resolve_target("opencode-01HZ"), "opencode-01HZ");
        assert_eq!(resolve_target("$3"), "$3");
        assert_eq!(resolve_target("  01HZ  "), "opencode-01HZ");
    }
}
