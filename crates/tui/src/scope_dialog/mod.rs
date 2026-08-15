//! Shared multi-select checkbox dialog for `inject_to` targets
//! (`parent` / `explore` / `build`), overlayed on the `/mcp` and `/cli` forms.
//!
//! Keys: ↑/↓ move, Space toggles, Enter confirms (empty selection keeps the
//! dialog open — at least one target is required), Esc cancels. Paste is
//! swallowed by the owning form while the dialog is open.

mod view;

pub use view::render_scope_dialog;

use crossterm::event::{KeyCode, KeyEvent};

use opencoder_core::InjectionTarget;

/// Selectable rows, in display order. Indices are the dialog cursor domain.
pub const OPTIONS: [&str; 3] = ["parent", "explore", "build"];

#[derive(Debug, PartialEq, Eq)]
pub enum ScopeOutcome {
    Idle,
    /// Apply the (non-empty) selection to the form's `inject_to`.
    Confirm(InjectionTarget),
    /// Discard changes and close.
    Cancel,
}

#[derive(Debug)]
pub struct ScopeDialog {
    target: InjectionTarget,
    /// Row index into [`OPTIONS`] (0..=2).
    cursor: usize,
}

impl ScopeDialog {
    pub fn new(target: InjectionTarget) -> Self {
        Self { target, cursor: 0 }
    }

    pub fn target(&self) -> InjectionTarget {
        self.target
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn checked(&self, idx: usize) -> bool {
        match idx {
            0 => self.target.parent,
            1 => self.target.explore,
            _ => self.target.build,
        }
    }

    pub fn any_checked(&self) -> bool {
        self.target.parent || self.target.explore || self.target.build
    }

    fn toggle_at(&mut self, idx: usize) {
        match idx {
            0 => self.target.parent = !self.target.parent,
            1 => self.target.explore = !self.target.explore,
            _ => self.target.build = !self.target.build,
        }
    }

    /// Handle one keystroke. Enter on an empty selection is rejected
    /// (returns `Idle`, dialog stays open) — an entry injected nowhere is
    /// indistinguishable from disabled, so it must be explicit via `enabled`.
    pub fn handle_key(&mut self, key: KeyEvent) -> ScopeOutcome {
        match key.code {
            KeyCode::Esc => ScopeOutcome::Cancel,
            KeyCode::Up => {
                self.cursor = (self.cursor + OPTIONS.len() - 1) % OPTIONS.len();
                ScopeOutcome::Idle
            }
            KeyCode::Down | KeyCode::Tab => {
                self.cursor = (self.cursor + 1) % OPTIONS.len();
                ScopeOutcome::Idle
            }
            KeyCode::Char(' ') => {
                self.toggle_at(self.cursor);
                ScopeOutcome::Idle
            }
            KeyCode::Enter => {
                if self.any_checked() {
                    ScopeOutcome::Confirm(self.target)
                } else {
                    ScopeOutcome::Idle
                }
            }
            _ => ScopeOutcome::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn subagent_only() -> InjectionTarget {
        InjectionTarget::subagents()
    }

    #[test]
    fn space_toggles_highlighted_row() {
        let mut d = ScopeDialog::new(subagent_only());
        assert!(!d.checked(0));
        d.handle_key(key(KeyCode::Char(' ')));
        assert!(d.checked(0), "cursor row 0 (parent) toggles on");
        assert!(d.any_checked());
    }

    #[test]
    fn arrows_wrap_around() {
        let mut d = ScopeDialog::new(InjectionTarget::parent_only());
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.cursor(), 2, "Up from row 0 wraps to last row");
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 0, "Down from last row wraps to row 0");
    }

    #[test]
    fn enter_confirms_non_empty_selection() {
        let mut d = ScopeDialog::new(subagent_only());
        assert_eq!(
            d.handle_key(key(KeyCode::Enter)),
            ScopeOutcome::Confirm(subagent_only())
        );
    }

    #[test]
    fn enter_on_empty_selection_keeps_dialog_open() {
        let mut d = ScopeDialog::new(InjectionTarget::parent_only());
        // uncheck the only box
        d.handle_key(key(KeyCode::Char(' ')));
        assert!(!d.any_checked());
        assert_eq!(d.handle_key(key(KeyCode::Enter)), ScopeOutcome::Idle);
        // re-check via space then confirm works again
        d.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(
            d.handle_key(key(KeyCode::Enter)),
            ScopeOutcome::Confirm(InjectionTarget::parent_only())
        );
    }

    #[test]
    fn escape_returns_cancel_even_after_toggles() {
        // The dialog keeps its in-progress state on cancel; the owning form
        // is what discards it (never reads `target()` on Cancel). That
        // discard path is covered by the cli/mcp form tests.
        let mut d = ScopeDialog::new(subagent_only());
        d.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(d.handle_key(key(KeyCode::Esc)), ScopeOutcome::Cancel);
    }

    #[test]
    fn full_pick_flow_updates_only_selected_subagent() {
        // Start parent-only, uncheck parent, check explore only.
        let mut d = ScopeDialog::new(InjectionTarget::parent_only());
        d.handle_key(key(KeyCode::Char(' '))); // uncheck parent
        d.handle_key(key(KeyCode::Down)); // row 1: explore
        d.handle_key(key(KeyCode::Char(' '))); // check explore
        let confirmed = match d.handle_key(key(KeyCode::Enter)) {
            ScopeOutcome::Confirm(t) => t,
            other => panic!("expected Confirm, got {other:?}"),
        };
        assert!(!confirmed.parent);
        assert!(confirmed.explore);
        assert!(!confirmed.build);
    }
}
