//! User-configurable keyboard shortcut specs (stored as strings like
//! `"ctrl+h"` in `opencoder.json`). The TUI crate parses these into
//! `KeyCombo` structs at startup; the merge layer applies them from JSON.

use serde::{Deserialize, Serialize};

/// Metadata for each bindable key: `(config_key, human_label)`.
/// Order matches the field order in [`KeymapConfig`].
pub const KEYMAP_INFO: &[(&str, &str)] = &[
    ("help", "Open shortcut settings"),
    ("quit", "Quit"),
    ("cancel", "Cancel running task / Quit when idle"),
    ("newline", "Insert newline"),
    ("cursor_home", "Cursor to line start"),
    ("cursor_end", "Cursor to line end"),
    ("delete_word", "Delete word backward"),
    ("clear_input", "Clear input"),
    ("switch_mode", "Toggle act/plan mode (keep context)"),
    ("paste_image", "Paste clipboard image"),
    ("undo", "Undo"),
    ("redo", "Redo"),
    ("forward_word", "Move word forward"),
    ("backward_word", "Move word backward"),
    ("collapse_blocks", "Collapse blocks + exit subagent"),
    ("force_redraw", "Force full-screen redraw"),
    ("copy_mode", "Toggle copy/selection mode"),
];

/// Configuration for all 17 re-bindable global keyboard shortcuts. Each field
/// holds a key-spec string parsed by the TUI's `parse_key_spec`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeymapConfig {
    pub help: String,
    pub quit: String,
    pub cancel: String,
    pub newline: String,
    pub cursor_home: String,
    pub cursor_end: String,
    pub delete_word: String,
    pub clear_input: String,
    #[serde(default = "default_switch_mode")]
    pub switch_mode: String,
    pub paste_image: String,
    pub undo: String,
    pub redo: String,
    pub forward_word: String,
    pub backward_word: String,
    pub collapse_blocks: String,
    pub force_redraw: String,
    pub copy_mode: String,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        KeymapConfig {
            help: "ctrl+h".into(),
            quit: "ctrl+d".into(),
            cancel: "ctrl+c".into(),
            newline: "ctrl+j".into(),
            cursor_home: "ctrl+a".into(),
            cursor_end: "ctrl+e".into(),
            delete_word: "ctrl+w".into(),
            clear_input: "ctrl+u".into(),
            switch_mode: default_switch_mode(),
            paste_image: "ctrl+v".into(),
            undo: "ctrl+z".into(),
            redo: "ctrl+y".into(),
            forward_word: "alt+f".into(),
            backward_word: "alt+b".into(),
            collapse_blocks: "ctrl+l".into(),
            force_redraw: "ctrl+f".into(),
            copy_mode: "ctrl+g".into(),
        }
    }
}

impl KeymapConfig {
    /// Look up the spec string for `key` (one of the `KEYMAP_INFO` keys).
    pub fn get(&self, key: &str) -> Option<&str> {
        Some(match key {
            "help" => &self.help,
            "quit" => &self.quit,
            "cancel" => &self.cancel,
            "newline" => &self.newline,
            "cursor_home" => &self.cursor_home,
            "cursor_end" => &self.cursor_end,
            "delete_word" => &self.delete_word,
            "clear_input" => &self.clear_input,
            "switch_mode" => &self.switch_mode,
            "paste_image" => &self.paste_image,
            "undo" => &self.undo,
            "redo" => &self.redo,
            "forward_word" => &self.forward_word,
            "backward_word" => &self.backward_word,
            "collapse_blocks" => &self.collapse_blocks,
            "force_redraw" => &self.force_redraw,
            "copy_mode" => &self.copy_mode,
            _ => return None,
        })
    }

    /// Set the spec string for `key`. Returns `false` if `key` is unknown.
    pub fn set(&mut self, key: &str, value: String) -> bool {
        match key {
            "help" => self.help = value,
            "quit" => self.quit = value,
            "cancel" => self.cancel = value,
            "newline" => self.newline = value,
            "cursor_home" => self.cursor_home = value,
            "cursor_end" => self.cursor_end = value,
            "delete_word" => self.delete_word = value,
            "clear_input" => self.clear_input = value,
            "switch_mode" => self.switch_mode = value,
            "paste_image" => self.paste_image = value,
            "undo" => self.undo = value,
            "redo" => self.redo = value,
            "forward_word" => self.forward_word = value,
            "backward_word" => self.backward_word = value,
            "collapse_blocks" => self.collapse_blocks = value,
            "force_redraw" => self.force_redraw = value,
            "copy_mode" => self.copy_mode = value,
            _ => return false,
        }
        true
    }
}

fn default_switch_mode() -> String {
    "ctrl+t".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_documented_defaults() {
        let d = KeymapConfig::default();
        assert_eq!(d.help, "ctrl+h");
        assert_eq!(d.quit, "ctrl+d");
        assert_eq!(d.cancel, "ctrl+c");
        assert_eq!(d.newline, "ctrl+j");
        assert_eq!(d.cursor_home, "ctrl+a");
        assert_eq!(d.cursor_end, "ctrl+e");
        assert_eq!(d.delete_word, "ctrl+w");
        assert_eq!(d.clear_input, "ctrl+u");
        assert_eq!(d.switch_mode, "ctrl+t");
        assert_eq!(d.paste_image, "ctrl+v");
        assert_eq!(d.undo, "ctrl+z");
        assert_eq!(d.redo, "ctrl+y");
        assert_eq!(d.forward_word, "alt+f");
        assert_eq!(d.backward_word, "alt+b");
        assert_eq!(d.collapse_blocks, "ctrl+l");
        assert_eq!(d.force_redraw, "ctrl+f");
        assert_eq!(d.copy_mode, "ctrl+g");
    }

    #[test]
    fn get_returns_correct_spec() {
        let d = KeymapConfig::default();
        assert_eq!(d.get("help"), Some("ctrl+h"));
        assert_eq!(d.get("force_redraw"), Some("ctrl+f"));
        assert_eq!(d.get("nonexistent"), None);
    }

    #[test]
    fn set_updates_value_and_round_trips() {
        let mut d = KeymapConfig::default();
        assert!(d.set("help", "f1".into()));
        assert_eq!(d.get("help"), Some("f1"));
        assert!(!d.set("nonexistent", "x".into()));
    }

    #[test]
    fn keymap_info_count_matches_fields() {
        let fields = serde_json::to_value(KeymapConfig::default())
            .expect("defaults serialize")
            .as_object()
            .expect("defaults are an object")
            .len();
        assert_eq!(KEYMAP_INFO.len(), 17);
        assert_eq!(
            KEYMAP_INFO.len(),
            fields,
            "KEYMAP_INFO must cover every field"
        );
    }

    /// Old user configs can still carry the retired Alt+Tab variant. Plain
    /// `Deserialize` ignores that unknown field while restoring the live
    /// `switch_mode` binding.
    #[test]
    fn legacy_keymap_restores_switch_mode_and_ignores_retired_variants() {
        let legacy = r#"{
            "help": "ctrl+h",
            "quit": "ctrl+d",
            "cancel": "ctrl+c",
            "newline": "ctrl+j",
            "cursor_home": "ctrl+a",
            "cursor_end": "ctrl+e",
            "delete_word": "ctrl+w",
            "clear_input": "ctrl+u",
            "switch_mode": "ctrl+t",
            "paste_image": "ctrl+v",
            "undo": "ctrl+z",
            "redo": "ctrl+y",
            "forward_word": "alt+f",
            "backward_word": "alt+b",
            "switch_mode_clear": "alt+tab",
            "collapse_blocks": "ctrl+l",
            "force_redraw": "ctrl+f",
            "copy_mode": "ctrl+g"
        }"#;
        let cfg: KeymapConfig = serde_json::from_str(legacy).expect("legacy keymap loads");
        assert_eq!(cfg, KeymapConfig::default());
        assert_eq!(cfg.get("switch_mode"), Some("ctrl+t"));
        assert!(cfg.get("switch_mode_clear").is_none());
    }

    #[test]
    fn keymap_without_switch_mode_uses_ctrl_t_default() {
        let json = serde_json::to_string(&KeymapConfig::default()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value.as_object_mut().unwrap().remove("switch_mode");
        let cfg: KeymapConfig = serde_json::from_value(value).expect("older keymap loads");
        assert_eq!(cfg.switch_mode, "ctrl+t");
    }
}
