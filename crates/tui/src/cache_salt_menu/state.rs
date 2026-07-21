//! State + keystroke handling for the `/cache_salt` read-only panel.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_store::Store;

/// One row in the salt panel.
#[derive(Debug, Clone)]
pub struct CacheSaltEntry {
    /// "main" for the parent, otherwise the subagent kind (explore/build/...).
    pub role: String,
    /// The salt string `<agent_name>:<session_id>` sent on requests.
    pub salt: String,
    /// Lifecycle tag for subagents ("running"/"completed"/...); empty for main.
    pub status: String,
    /// True for the parent entry (rendered first + highlighted with `*`).
    pub is_parent: bool,
}

/// Outcome of a keystroke while the panel is open.
#[derive(Debug, PartialEq, Eq)]
pub enum CacheSaltOutcome {
    Idle,
    Cancel,
    Quit,
}

/// Read-only panel state. `entries[0]` is always the parent (main) session.
pub struct CacheSaltMenu {
    /// Mirrors `config.cache_salt == Some(true)`.
    pub enabled: bool,
    pub entries: Vec<CacheSaltEntry>,
}

impl CacheSaltMenu {
    /// Build the panel: parent salt first, then every subagent of `session_id`
    /// (via the store) in insertion (seq) order. `agent_name` is the main
    /// session's agent name.
    pub async fn build(
        store: &dyn Store,
        session_id: &str,
        agent_name: &str,
        enabled: bool,
    ) -> Result<Self> {
        let mut entries = vec![CacheSaltEntry {
            role: "main".to_string(),
            salt: format!("{agent_name}:{session_id}"),
            status: String::new(),
            is_parent: true,
        }];
        for t in store.list_subagent_tasks(session_id).await? {
            entries.push(CacheSaltEntry {
                role: t.agent.clone(),
                salt: format!("{}:{}", t.agent, t.child_session_id),
                status: t.status.as_str().to_string(),
                is_parent: false,
            });
        }
        Ok(Self { enabled, entries })
    }

    /// Fallback constructor (no store / query failed): parent row only.
    pub fn parent_only(agent_name: &str, session_id: &str, enabled: bool) -> Self {
        Self {
            enabled,
            entries: vec![CacheSaltEntry {
                role: "main".to_string(),
                salt: format!("{agent_name}:{session_id}"),
                status: String::new(),
                is_parent: true,
            }],
        }
    }
}

/// Read-only panel: Esc/Enter close, Ctrl+D quits the app, all else idle.
/// Closing sets the `Option` to `None` so the caller drops modal mode.
pub fn handle_cache_salt_key(menu: &mut Option<CacheSaltMenu>, k: KeyEvent) -> CacheSaltOutcome {
    if menu.is_none() {
        return CacheSaltOutcome::Idle;
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            *menu = None;
            return CacheSaltOutcome::Quit;
        }
        return CacheSaltOutcome::Idle;
    }
    match k.code {
        KeyCode::Esc | KeyCode::Enter => {
            *menu = None;
            CacheSaltOutcome::Cancel
        }
        _ => CacheSaltOutcome::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_only_has_single_parent_row() {
        let m = CacheSaltMenu::parent_only("act", "sess-1", true);
        assert!(m.enabled);
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].salt, "act:sess-1");
        assert_eq!(m.entries[0].role, "main");
        assert!(m.entries[0].is_parent);
        assert!(m.entries[0].status.is_empty());
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn ctrl_d() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
    }
    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    #[test]
    fn esc_closes_panel() {
        let mut m = Some(CacheSaltMenu::parent_only("act", "s", true));
        assert_eq!(handle_cache_salt_key(&mut m, esc()), CacheSaltOutcome::Cancel);
        assert!(m.is_none(), "Esc must close (set Option to None)");
    }

    #[test]
    fn enter_closes_panel() {
        let mut m = Some(CacheSaltMenu::parent_only("act", "s", true));
        assert_eq!(handle_cache_salt_key(&mut m, enter()), CacheSaltOutcome::Cancel);
        assert!(m.is_none());
    }

    #[test]
    fn ctrl_d_quits_and_closes() {
        let mut m = Some(CacheSaltMenu::parent_only("act", "s", true));
        assert_eq!(handle_cache_salt_key(&mut m, ctrl_d()), CacheSaltOutcome::Quit);
        assert!(m.is_none());
    }

    #[test]
    fn other_keys_keep_panel_open() {
        let mut m = Some(CacheSaltMenu::parent_only("act", "s", true));
        assert_eq!(handle_cache_salt_key(&mut m, up()), CacheSaltOutcome::Idle);
        assert!(m.is_some(), "Idle key must NOT close the panel");
    }

    #[test]
    fn handles_none_menu_gracefully() {
        let mut m: Option<CacheSaltMenu> = None;
        assert_eq!(handle_cache_salt_key(&mut m, esc()), CacheSaltOutcome::Idle);
    }
}
