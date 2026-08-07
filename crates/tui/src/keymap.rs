//! Keymap engine: parses key-spec strings (e.g. `"ctrl+h"`) into `KeyCombo`
//! structs that can match `KeyEvent` values. Used by `key_handler.rs` and
//! `app_helpers.rs` to drive all 18 re-bindable shortcuts from config.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, KeymapConfig};

/// A parsed key combination: modifiers + a key code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyCombo {
    pub mods: KeyModifiers,
    pub code: KeyCode,
}

/// Compute the raw control char for a lowercase ASCII letter:
/// `'a'` → `'\u{1}'`, `'d'` → `'\u{4}'`, etc.
fn ctrl_char(c: char) -> Option<char> {
    if c.is_ascii_lowercase() {
        Some((c as u8 - b'a' + 1) as char)
    } else {
        None
    }
}

/// Reverse of `ctrl_char`: `'\u{4}'` → `'d'`.
fn letter_from_ctrl(c: char) -> Option<char> {
    let b = c as u32;
    if (1..=26).contains(&b) {
        Some((b'a' as u32 + b - 1) as u8 as char)
    } else {
        None
    }
}

impl KeyCombo {
    /// Returns `true` if `ev` matches this key combination. Handles:
    /// - Exact match (same modifiers + code).
    /// - Raw control chars: `Ctrl+D` matches both `Char('d')+CONTROL` and
    ///   `Char('\u{4}')` with or without the CONTROL flag.
    /// - Case-insensitive letter matching (Alt+F matches both 'f' and 'F').
    /// - Tab/BackTab equivalence (BackTab ≡ Tab+SHIFT).
    pub(crate) fn matches(&self, ev: &KeyEvent) -> bool {
        // --- Normalize BackTab → Tab+SHIFT on both sides ---
        let (self_mods, self_code) = normalize(self.mods, self.code);
        let (ev_mods, ev_code) = normalize(ev.modifiers, ev.code);

        // --- Exact normalized match ---
        if self_mods == ev_mods && self_code == ev_code {
            return true;
        }

        // --- Tab/BackTab: after normalization both are Tab, but BackTab
        // adds SHIFT. When the spec omits SHIFT, allow it on the event
        // side so "alt+tab" matches terminal BackTab+Alt.
        if self_code == KeyCode::Tab
            && ev_code == KeyCode::Tab
            && !self_mods.contains(KeyModifiers::SHIFT)
            && ev_mods.contains(KeyModifiers::SHIFT)
            && (self_mods & !KeyModifiers::SHIFT) == (ev_mods & !KeyModifiers::SHIFT)
        {
            return true;
        }

        // --- Char comparisons (case-insensitive + raw control char) ---
        if let (KeyCode::Char(sc), KeyCode::Char(ec)) = (self_code, ev_code) {
            let sc_lower = sc.to_ascii_lowercase();
            let ec_lower = ec.to_ascii_lowercase();

            // Raw control char: combo is ctrl+letter, event is the raw \u{N}
            if self_mods.contains(KeyModifiers::CONTROL) {
                if let Some(raw) = ctrl_char(sc_lower) {
                    if ec == raw {
                        return true;
                    }
                }
            }

            // Case-insensitive letter match with compatible modifiers
            if sc_lower == ec_lower {
                // For ctrl combos: both sides must have CONTROL
                if self_mods.contains(KeyModifiers::CONTROL)
                    && ev_mods.contains(KeyModifiers::CONTROL)
                    && !self_mods.contains(KeyModifiers::ALT)
                    && !ev_mods.contains(KeyModifiers::ALT)
                {
                    return true;
                }
                // For alt combos (no ctrl): both sides must have ALT
                if self_mods.contains(KeyModifiers::ALT)
                    && ev_mods.contains(KeyModifiers::ALT)
                    && !self_mods.contains(KeyModifiers::CONTROL)
                    && !ev_mods.contains(KeyModifiers::CONTROL)
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Normalize BackTab → (Tab, mods|SHIFT). Other codes pass through unchanged.
fn normalize(mods: KeyModifiers, code: KeyCode) -> (KeyModifiers, KeyCode) {
    match code {
        KeyCode::BackTab => (mods | KeyModifiers::SHIFT, KeyCode::Tab),
        _ => (mods, code),
    }
}

/// Parse a key-spec string like `"ctrl+h"`, `"alt+tab"`, `"ctrl+shift+tab"`
/// into a `KeyCombo`. Returns `None` on unparseable input.
pub(crate) fn parse_key_spec(spec: &str) -> Option<KeyCombo> {
    let spec = spec.trim().to_lowercase();
    let parts: Vec<&str> = spec.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut mods = KeyModifiers::NONE;
    let mut key_part: Option<&str> = None;

    for part in &parts {
        match *part {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "option" | "opt" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            "super" | "meta" | "cmd" | "win" => mods |= KeyModifiers::SUPER,
            "" => {}
            other => {
                if key_part.is_some() {
                    return None; // multiple non-modifier parts
                }
                key_part = Some(other);
            }
        }
    }

    let key = key_part?;
    let code = parse_key_name(key)?;

    // Normalize shift+tab → BackTab (without redundant SHIFT flag)
    if mods.contains(KeyModifiers::SHIFT) && code == KeyCode::Tab {
        return Some(KeyCombo {
            mods: mods & !KeyModifiers::SHIFT,
            code: KeyCode::BackTab,
        });
    }

    Some(KeyCombo { mods, code })
}

/// Parse a single key name (no modifiers) into a `KeyCode`.
fn parse_key_name(name: &str) -> Option<KeyCode> {
    Some(match name {
        "enter" | "return" | "ret" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" | "stab" | "btab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "esc" | "escape" => KeyCode::Esc,
        "space" | "spc" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        // Single character
        c if c.chars().count() == 1 => {
            let ch = c.chars().next().unwrap();
            KeyCode::Char(ch)
        }
        _ => return None,
    })
}

/// Convert a `KeyEvent` back into a spec string. Used in capture mode of the
/// keymap menu. Returns `None` for keys that cannot be represented as specs
/// (e.g. null key, unknown function keys).
pub(crate) fn key_event_to_spec(k: KeyEvent) -> Option<String> {
    let (mods, code) = normalize(k.modifiers, k.code);

    let mut parts: Vec<&str> = Vec::new();
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }
    if mods.contains(KeyModifiers::SUPER) {
        parts.push("super");
    }

    let key_str: String = match code {
        KeyCode::Char(c) => {
            // Raw control char → convert to ctrl+letter
            if c.is_ascii_control() {
                if let Some(letter) = letter_from_ctrl(c) {
                    if !parts.contains(&"ctrl") {
                        parts.push("ctrl");
                    }
                    return Some(format!("{}+{}", parts.join("+"), letter));
                }
                return None; // non-letter control char
            }
            c.to_string()
        }
        KeyCode::Enter => "enter".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::F(n) => format!("f{}", n),
        _ => return None,
    };

    parts.push(&key_str);
    Some(parts.join("+"))
}

/// All 18 parsed key bindings, ready for O(1) matching in the event loop.
pub(crate) struct KeyBindings {
    pub help: KeyCombo,
    pub quit: KeyCombo,
    pub cancel: KeyCombo,
    pub newline: KeyCombo,
    pub cursor_home: KeyCombo,
    pub cursor_end: KeyCombo,
    pub delete_word: KeyCombo,
    pub clear_input: KeyCombo,
    pub switch_mode: KeyCombo,
    pub paste_image: KeyCombo,
    pub undo: KeyCombo,
    pub redo: KeyCombo,
    pub forward_word: KeyCombo,
    pub backward_word: KeyCombo,
    pub switch_mode_clear: KeyCombo,
    pub switch_mode_keep: KeyCombo,
    pub collapse_blocks: KeyCombo,
    pub force_redraw: KeyCombo,
}

impl KeyBindings {
    /// Build from a [`Config`], falling back to the default spec for any
    /// field whose configured value fails to parse.
    pub(crate) fn from_config(config: &Config) -> Self {
        let km = &config.keymap;
        let d = KeymapConfig::default();
        KeyBindings {
            help: parse_or_default(&km.help, &d.help),
            quit: parse_or_default(&km.quit, &d.quit),
            cancel: parse_or_default(&km.cancel, &d.cancel),
            newline: parse_or_default(&km.newline, &d.newline),
            cursor_home: parse_or_default(&km.cursor_home, &d.cursor_home),
            cursor_end: parse_or_default(&km.cursor_end, &d.cursor_end),
            delete_word: parse_or_default(&km.delete_word, &d.delete_word),
            clear_input: parse_or_default(&km.clear_input, &d.clear_input),
            switch_mode: parse_or_default(&km.switch_mode, &d.switch_mode),
            paste_image: parse_or_default(&km.paste_image, &d.paste_image),
            undo: parse_or_default(&km.undo, &d.undo),
            redo: parse_or_default(&km.redo, &d.redo),
            forward_word: parse_or_default(&km.forward_word, &d.forward_word),
            backward_word: parse_or_default(&km.backward_word, &d.backward_word),
            switch_mode_clear: parse_or_default(&km.switch_mode_clear, &d.switch_mode_clear),
            switch_mode_keep: parse_or_default(&km.switch_mode_keep, &d.switch_mode_keep),
            collapse_blocks: parse_or_default(&km.collapse_blocks, &d.collapse_blocks),
            force_redraw: parse_or_default(&km.force_redraw, &d.force_redraw),
        }
    }
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

/// Parse `spec`; on failure, parse `default_spec` (which must always succeed).
fn parse_or_default(spec: &str, default_spec: &str) -> KeyCombo {
    parse_key_spec(spec)
        .or_else(|| parse_key_spec(default_spec))
        .unwrap_or_else(|| panic!("default keymap spec '{default_spec}' failed to parse"))
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod tests;
