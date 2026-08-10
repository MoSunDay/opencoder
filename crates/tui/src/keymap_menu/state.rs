//! State + keystroke handling for the keymap re-bind modal (Ctrl+H).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_core::{KeymapConfig, KEYMAP_INFO};

/// Number of buttons in the bottom button bar.
const BUTTON_COUNT: usize = 3;

/// Outcome of a keystroke while the keymap modal is open.
#[derive(Debug, PartialEq, Eq)]
pub enum KeymapOutcome {
    Idle,
    /// Close without saving.
    Cancel,
    /// Close and save: a JSON patch `{"keymap":{...}}` with only changed fields.
    Save(serde_json::Value),
    /// Close + quit the app.
    Quit,
}

/// Which element inside the keymap modal currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The shortcut list (navigation / re-binding).
    List,
    /// The bottom button bar (退出 / 恢复默认 / 帮助).
    Buttons,
}

/// Modal state for the keymap rebinding menu.
pub struct KeymapMenu {
    /// Index of the highlighted row.
    pub selected: usize,
    /// When `true`, the next key event is captured as the new binding for
    /// `selected` instead of navigating.
    pub capturing: bool,
    /// `(config_key, human_label, current_spec)` for all 21 entries.
    entries: Vec<(String, String, String)>,
    /// Original specs at construction time, for dirty detection.
    original_specs: Vec<String>,
    /// Currently focused element.
    focus: Focus,
    /// 0 = Exit, 1 = Help.
    selected_button: usize,
    /// `true` while the help overlay is open on top of the modal.
    help_open: bool,
    /// Scroll offset of the help overlay.
    help_scroll: u16,
    /// `true` while the reset-confirmation dialog is open on top of the modal.
    confirm_reset: bool,
}

impl KeymapMenu {
    /// Build from the current `KeymapConfig`.
    pub fn new(config: &KeymapConfig) -> Self {
        let entries: Vec<(String, String, String)> = KEYMAP_INFO
            .iter()
            .map(|(key, label)| {
                let spec = config.get(key).unwrap_or("").to_string();
                (key.to_string(), label.to_string(), spec)
            })
            .collect();
        let original_specs = entries.iter().map(|(_, _, s)| s.clone()).collect();
        KeymapMenu {
            selected: 0,
            capturing: false,
            entries,
            original_specs,
            focus: Focus::List,
            selected_button: 0,
            help_open: false,
            help_scroll: 0,
            confirm_reset: false,
        }
    }

    /// Read-only access to the display entries: `(key, label, spec)`.
    pub fn entries(&self) -> &[(String, String, String)] {
        &self.entries
    }

    /// `true` when any current spec differs from its original.
    pub fn is_dirty(&self) -> bool {
        self.entries
            .iter()
            .zip(self.original_specs.iter())
            .any(|((_, _, cur), orig)| cur != orig)
    }

    /// `(key, label, spec)` for the currently selected entry.
    pub fn selected_entry(&self) -> Option<&(String, String, String)> {
        self.entries.get(self.selected)
    }

    /// Number of rows.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Which element currently has keyboard focus.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// 0 = Exit, 1 = Reset, 2 = Help.
    pub fn selected_button(&self) -> usize {
        self.selected_button
    }

    /// `true` while the help overlay is visible.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// Current scroll offset of the help overlay.
    pub fn help_scroll(&self) -> u16 {
        self.help_scroll
    }

    /// `true` while the reset-confirmation dialog is visible.
    pub fn confirm_reset_open(&self) -> bool {
        self.confirm_reset
    }

    /// Move selection up (wraps around).
    fn move_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + self.entries.len() - 1) % self.entries.len();
    }

    /// Move selection down (wraps around).
    fn move_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.entries.len();
    }

    /// Cycle the button selection forward (Exit → Reset → Help → ...).
    fn next_button(&mut self) {
        self.selected_button = (self.selected_button + 1) % BUTTON_COUNT;
    }

    /// Cycle the button selection backward (Help → Reset → Exit → ...).
    fn prev_button(&mut self) {
        self.selected_button = (self.selected_button + BUTTON_COUNT - 1) % BUTTON_COUNT;
    }

    /// Set the spec for the selected entry (during capture mode).
    fn set_selected_spec(&mut self, spec: String) {
        if let Some((_, _, cur)) = self.entries.get_mut(self.selected) {
            *cur = spec;
        }
        self.capturing = false;
    }

    /// Build a JSON patch containing only the changed keymap fields.
    /// Returns `{"keymap": {"key": "spec", ...}}`.
    pub fn build_patch(&self) -> serde_json::Value {
        let changed: serde_json::Map<String, serde_json::Value> = self
            .entries
            .iter()
            .zip(self.original_specs.iter())
            .filter(|((_, _, cur), orig)| cur != *orig)
            .map(|((key, _, cur), _)| (key.clone(), serde_json::Value::String(cur.clone())))
            .collect();
        serde_json::json!({ "keymap": changed })
    }

    /// Reset all entries to their default key bindings.
    pub fn reset_to_defaults(&mut self) {
        let d = KeymapConfig::default();
        for (key, _, spec) in &mut self.entries {
            if let Some(v) = d.get(key) {
                *spec = v.to_string();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_selected_spec_for_test(&mut self, idx: usize, spec: &str) {
        if idx < self.entries.len() {
            self.selected = idx;
            self.set_selected_spec(spec.to_string());
        }
    }
}

/// Close-and-save-or-cancel helper shared by Esc, Ctrl+D and the Exit button.
/// If dirty, returns `Save(patch)`; otherwise returns `fallback`.
fn close_with_save(menu: &mut Option<KeymapMenu>, fallback: KeymapOutcome) -> KeymapOutcome {
    let m = menu.as_mut().unwrap();
    let dirty = m.is_dirty();
    let patch = if dirty { Some(m.build_patch()) } else { None };
    *menu = None;
    if let Some(p) = patch {
        KeymapOutcome::Save(p)
    } else {
        fallback
    }
}

/// Handle a key event while the keymap modal is open.
/// On `Save`/`Cancel`/`Quit`, the caller closes the modal (`*menu = None`).
pub fn handle_keymap_key(menu: &mut Option<KeymapMenu>, k: KeyEvent) -> KeymapOutcome {
    let Some(m) = menu.as_mut() else {
        return KeymapOutcome::Idle;
    };

    // --- Confirm-reset dialog open: intercept all keys ---
    if m.confirm_reset {
        match k.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                m.confirm_reset = false;
                m.reset_to_defaults();
                return KeymapOutcome::Idle;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                m.confirm_reset = false;
                return KeymapOutcome::Idle;
            }
            _ => return KeymapOutcome::Idle,
        }
    }

    // --- Help overlay open: only scroll + Esc/close ---
    if m.help_open {
        match k.code {
            KeyCode::Up => m.help_scroll = m.help_scroll.saturating_sub(1),
            KeyCode::Down => m.help_scroll = m.help_scroll.saturating_add(1),
            KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                m.help_scroll = m.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                m.help_scroll = m.help_scroll.saturating_sub(1);
            }
            KeyCode::Esc => {
                m.help_open = false;
                m.help_scroll = 0;
            }
            _ => {}
        }
        return KeymapOutcome::Idle;
    }

    // --- Capture mode: next key becomes the new binding ---
    if m.capturing {
        // Esc cancels capture without changing the binding
        if k.code == KeyCode::Esc {
            m.capturing = false;
            return KeymapOutcome::Idle;
        }
        // Ctrl+D / raw EOT quits the app even in capture mode
        if k.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
        {
            *menu = None;
            return KeymapOutcome::Quit;
        }
        // Convert the key event to a spec string
        if let Some(spec) = crate::keymap::key_event_to_spec(k) {
            m.set_selected_spec(spec);
        }
        // If the key couldn't be converted, stay in capture mode
        return KeymapOutcome::Idle;
    }

    // --- Global shortcuts (work regardless of focus) ---
    match k.code {
        KeyCode::Tab => {
            m.focus = match m.focus {
                Focus::List => Focus::Buttons,
                Focus::Buttons => Focus::List,
            };
            return KeymapOutcome::Idle;
        }
        KeyCode::Esc => {
            return close_with_save(menu, KeymapOutcome::Cancel);
        }
        KeyCode::Char('d') | KeyCode::Char('\u{4}')
            if k.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            return close_with_save(menu, KeymapOutcome::Quit);
        }
        _ => {}
    }

    // --- Focus-specific handling ---
    match m.focus {
        Focus::List => match k.code {
            KeyCode::Up => m.move_up(),
            KeyCode::Down => m.move_down(),
            KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => m.move_down(),
            KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => m.move_up(),
            KeyCode::Enter => {
                m.capturing = true;
            }
            KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                m.reset_to_defaults();
            }
            _ => {}
        },
        Focus::Buttons => match k.code {
            KeyCode::Left | KeyCode::Char('k')
                if k.code == KeyCode::Left || k.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                m.prev_button();
            }
            KeyCode::Right | KeyCode::Char('j')
                if k.code == KeyCode::Right || k.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                m.next_button();
            }
            KeyCode::Enter => {
                let button = m.selected_button;
                match button {
                    0 => return close_with_save(menu, KeymapOutcome::Quit),
                    1 => m.confirm_reset = true,
                    _ => {
                        m.help_open = true;
                        m.help_scroll = 0;
                    }
                }
            }
            KeyCode::Up | KeyCode::Down => {
                // Move focus back to the list.
                m.focus = Focus::List;
            }
            _ => {}
        },
    }
    KeymapOutcome::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_menu() -> KeymapMenu {
        KeymapMenu::new(&KeymapConfig::default())
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn new_menu_has_20_entries() {
        let m = make_menu();
        assert_eq!(m.len(), 20);
        assert_eq!(m.selected, 0);
        assert!(!m.capturing);
        assert!(!m.is_dirty());
        assert_eq!(m.focus(), Focus::List);
        assert_eq!(m.selected_button(), 0);
        assert!(!m.help_open());
    }

    #[test]
    fn navigate_down_wraps() {
        let mut m = make_menu();
        m.move_down();
        assert_eq!(m.selected, 1);
        // Wrap to 0 from last
        m.selected = 19;
        m.move_down();
        assert_eq!(m.selected, 0);
    }

    #[test]
    fn navigate_up_wraps() {
        let mut m = make_menu();
        m.move_up(); // from 0 -> last index
        assert_eq!(m.selected, 19);
    }

    #[test]
    fn enter_starts_capture() {
        let mut menu = Some(make_menu());
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().capturing);
    }

    #[test]
    fn capture_sets_spec_and_exits_capture() {
        let mut menu = Some(make_menu());
        // Enter to start capture
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().capturing);
        // Press F1
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        // Capture mode exited
        assert!(!menu.as_ref().unwrap().capturing);
        // First entry spec is now f1
        let entries = menu.as_ref().unwrap().entries();
        assert_eq!(entries[0].2, "f1");
        // Menu is dirty
        assert!(menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn capture_esc_cancels_without_change() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!menu.as_ref().unwrap().capturing);
        assert!(!menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn save_patch_contains_only_changed_fields() {
        let mut menu = Some(make_menu());
        // Capture F1 for the first entry (help)
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        // Escape to save
        let out = handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        match out {
            KeymapOutcome::Save(v) => {
                let km = v.get("keymap").unwrap().as_object().unwrap();
                assert_eq!(km.len(), 1);
                assert_eq!(km.get("help").unwrap(), "f1");
            }
            _ => panic!("expected Save, got {:?}", out),
        }
    }

    #[test]
    fn esc_without_changes_returns_cancel() {
        let mut menu = Some(make_menu());
        let out = handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Cancel);
        assert!(menu.is_none());
    }

    #[test]
    fn ctrl_d_quits_without_changes() {
        let mut menu = Some(make_menu());
        let out = handle_keymap_key(&mut menu, key(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(out, KeymapOutcome::Quit);
        assert!(menu.is_none());
    }

    #[test]
    fn reset_to_defaults_restores_original() {
        let mut menu = make_menu();
        // Change a binding
        let key = menu.entries()[0].0.clone();
        let _ = key;
        menu.set_selected_spec_for_test(0, "f1");
        assert!(menu.is_dirty());
        // Reset
        menu.reset_to_defaults();
        assert!(!menu.is_dirty());
        // All entries match defaults
        let d = KeymapConfig::default();
        for (k, _, spec) in menu.entries().iter() {
            assert_eq!(*spec, d.get(k).unwrap());
        }
    }

    #[test]
    fn ctrl_r_resets_to_defaults() {
        let mut menu = Some(make_menu());
        // Start capturing for entry 0
        let _ = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        // Press F1 to set a binding
        let _ = handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Press Ctrl+R to reset
        let outcome = handle_keymap_key(&mut menu, key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(outcome, KeymapOutcome::Idle);
        assert!(!menu.as_ref().unwrap().is_dirty());
    }

    // --- New tests for button bar + help overlay ---

    #[test]
    fn tab_toggles_focus_list_to_buttons() {
        let mut menu = Some(make_menu());
        assert_eq!(menu.as_ref().unwrap().focus(), Focus::List);
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().focus(), Focus::Buttons);
    }

    #[test]
    fn tab_toggles_focus_buttons_to_list() {
        let mut menu = Some(make_menu());
        // List → Buttons
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Buttons → List
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().focus(), Focus::List);
    }

    #[test]
    fn left_right_navigate_buttons() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
        // Right → button 1 (Reset)
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
        // Right → button 2 (Help)
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 2);
        // Right wraps → button 0 (Exit)
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
        // Left wraps backward → button 2 (Help)
        handle_keymap_key(&mut menu, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 2);
        // Left → button 1 (Reset)
        handle_keymap_key(&mut menu, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
    }

    #[test]
    fn prev_button_goes_backward() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Start at 0 (Exit)
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
        // Left wraps to last button (2 = Help)
        handle_keymap_key(&mut menu, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 2);
        // Left → 1 (Reset)
        handle_keymap_key(&mut menu, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
        // Left → 0 (Exit)
        handle_keymap_key(&mut menu, key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
    }

    #[test]
    fn ctrl_j_ctrl_k_navigate_buttons() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
        // Ctrl+J → next (button 1)
        handle_keymap_key(&mut menu, key(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
        // Ctrl+J → next (button 2)
        handle_keymap_key(&mut menu, key(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 2);
        // Ctrl+K → prev (button 1)
        handle_keymap_key(&mut menu, key(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
        // Ctrl+K → prev (button 0)
        handle_keymap_key(&mut menu, key(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 0);
    }

    #[test]
    fn exit_button_quits_without_changes() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Button 0 = Exit, Enter activates
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Quit);
        assert!(menu.is_none());
    }

    #[test]
    fn exit_button_saves_when_dirty_then_quits() {
        let mut menu = Some(make_menu());
        // Make a change
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Tab to buttons
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Press Enter on Exit (button 0)
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        match out {
            KeymapOutcome::Save(v) => {
                assert_eq!(v.get("keymap").unwrap().get("help").unwrap(), "f1");
            }
            _ => panic!("expected Save, got {:?}", out),
        }
        assert!(menu.is_none());
    }

    #[test]
    fn help_button_opens_overlay() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Navigate to Help (button 2): Right twice
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 2);
        // Activate
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().help_open());
    }

    #[test]
    fn help_overlay_scroll_down_increments() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().help_open());
        let before = menu.as_ref().unwrap().help_scroll();
        handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().help_scroll(), before + 1);
    }

    #[test]
    fn help_overlay_scroll_up_decrements() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        // Scroll down a few, then up
        handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().help_scroll(), 2);
    }

    #[test]
    fn help_overlay_esc_closes() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().help_open());
        handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!menu.as_ref().unwrap().help_open());
        assert_eq!(menu.as_ref().unwrap().help_scroll(), 0);
        // Menu is still open (only help overlay closed)
        assert!(menu.is_some());
    }

    #[test]
    fn help_overlay_esc_does_not_close_modal() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        // Esc in help mode → closes overlay, NOT modal
        let out = handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.is_some());
    }

    #[test]
    fn up_down_in_buttons_focus_returns_to_list() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().focus(), Focus::Buttons);
        handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().focus(), Focus::List);
    }

    // --- Reset confirmation dialog tests ---

    #[test]
    fn reset_button_opens_confirm() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        // Navigate to Reset (button 1)
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(menu.as_ref().unwrap().selected_button(), 1);
        assert!(!menu.as_ref().unwrap().confirm_reset_open());
        // Activate
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().confirm_reset_open());
    }

    #[test]
    fn confirm_enter_resets_defaults() {
        let mut menu = Some(make_menu());
        // Make a change first
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Open confirm dialog via button 1
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().confirm_reset_open());
        // Confirm with Enter
        let out = handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(!menu.as_ref().unwrap().confirm_reset_open());
        assert!(!menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn confirm_y_also_resets() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Open confirm dialog
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        // Confirm with 'y'
        handle_keymap_key(&mut menu, key(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(!menu.as_ref().unwrap().confirm_reset_open());
        assert!(!menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn confirm_esc_cancels() {
        let mut menu = Some(make_menu());
        // Make a change
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Open confirm dialog
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().confirm_reset_open());
        // Cancel with Esc
        let out = handle_keymap_key(&mut menu, key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(!menu.as_ref().unwrap().confirm_reset_open());
        // Changes still present (not reset)
        assert!(menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn confirm_n_also_cancels() {
        let mut menu = Some(make_menu());
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::F(1), KeyModifiers::NONE));
        // Open confirm dialog
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().confirm_reset_open());
        // Cancel with 'n'
        handle_keymap_key(&mut menu, key(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(!menu.as_ref().unwrap().confirm_reset_open());
        // Changes still present
        assert!(menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn confirm_intercepts_other_keys() {
        let mut menu = Some(make_menu());
        // Open confirm dialog
        handle_keymap_key(&mut menu, key(KeyCode::Tab, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Right, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().confirm_reset_open());
        // Other keys are intercepted (ignored)
        let out = handle_keymap_key(&mut menu, key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().confirm_reset_open());
    }
}
