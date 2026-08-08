//! State + keystroke handling for the keymap re-bind modal (Ctrl+H).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_core::{KeymapConfig, KEYMAP_INFO};

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

/// Modal state for the keymap rebinding menu.
pub struct KeymapMenu {
    /// Index of the highlighted row.
    pub selected: usize,
    /// When `true`, the next key event is captured as the new binding for
    /// `selected` instead of navigating.
    pub capturing: bool,
    /// `(config_key, human_label, current_spec)` for all 18 entries.
    entries: Vec<(String, String, String)>,
    /// Original specs at construction time, for dirty detection.
    original_specs: Vec<String>,
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

/// Handle a key event while the keymap modal is open.
/// On `Save`/`Cancel`/`Quit`, the caller closes the modal (`*menu = None`).
pub fn handle_keymap_key(menu: &mut Option<KeymapMenu>, k: KeyEvent) -> KeymapOutcome {
    let Some(m) = menu.as_mut() else {
        return KeymapOutcome::Idle;
    };

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

    // --- Navigation mode ---
    match k.code {
        KeyCode::Up => m.move_up(),
        KeyCode::Down => m.move_down(),
        KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => m.move_down(),
        KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => m.move_up(),
        KeyCode::Enter => {
            m.capturing = true;
        }
        KeyCode::Esc => {
            let dirty = m.is_dirty();
            let patch = if dirty { Some(m.build_patch()) } else { None };
            *menu = None;
            if let Some(p) = patch {
                return KeymapOutcome::Save(p);
            }
            return KeymapOutcome::Cancel;
        }
        KeyCode::Char('d') | KeyCode::Char('\u{4}')
            if k.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let dirty = m.is_dirty();
            let patch = if dirty { Some(m.build_patch()) } else { None };
            *menu = None;
            if let Some(p) = patch {
                return KeymapOutcome::Save(p);
            }
            return KeymapOutcome::Quit;
        }
        KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            m.reset_to_defaults();
        }
        _ => {}
    }
    KeymapOutcome::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_menu() -> KeymapMenu {
        KeymapMenu::new(&KeymapConfig::default())
    }

    #[test]
    fn new_menu_has_18_entries() {
        let m = make_menu();
        assert_eq!(m.len(), 18);
        assert_eq!(m.selected, 0);
        assert!(!m.capturing);
        assert!(!m.is_dirty());
    }

    #[test]
    fn navigate_down_wraps() {
        let mut m = make_menu();
        m.move_down();
        assert_eq!(m.selected, 1);
        // Wrap to 0 from last
        m.selected = 17;
        m.move_down();
        assert_eq!(m.selected, 0);
    }

    #[test]
    fn navigate_up_wraps() {
        let mut m = make_menu();
        m.move_up(); // from 0 → 17
        assert_eq!(m.selected, 17);
    }

    #[test]
    fn enter_starts_capture() {
        let mut menu = Some(make_menu());
        let out = handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().capturing);
    }

    #[test]
    fn capture_sets_spec_and_exits_capture() {
        let mut menu = Some(make_menu());
        // Enter to start capture
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().capturing);
        // Press F1
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
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
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!menu.as_ref().unwrap().capturing);
        assert!(!menu.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn save_patch_contains_only_changed_fields() {
        let mut menu = Some(make_menu());
        // Capture F1 for the first entry (help)
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        // Escape to save
        let out = handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
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
        let out = handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(out, KeymapOutcome::Cancel);
        assert!(menu.is_none());
    }

    #[test]
    fn ctrl_d_quits_without_changes() {
        let mut menu = Some(make_menu());
        let out = handle_keymap_key(
            &mut menu,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
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
        let _ = handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        // Press F1 to set a binding
        let _ = handle_keymap_key(&mut menu, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert!(menu.as_ref().unwrap().is_dirty());
        // Press Ctrl+R to reset
        let outcome = handle_keymap_key(
            &mut menu,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert_eq!(outcome, KeymapOutcome::Idle);
        assert!(!menu.as_ref().unwrap().is_dirty());
    }
}
