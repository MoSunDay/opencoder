//! Question dialog state machine — pure, no I/O. `app.rs` owns
//! `Option<QuestionMenu>`; keys map to [`QuestionAction`]s the caller turns
//! into hub resolutions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use ratatui::text::Line;

/// One pending question parsed from a `question` ToolStart event.
#[derive(Debug, Clone)]
pub struct QuestionPrompt {
    /// Tool-call id — the `QuestionHub` key and the ToolEnd match key.
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
}

/// Which row of the dialog owns key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionFocus {
    /// ↑/↓ over the preset options (+ the custom row).
    Options,
    /// Free-text entry (the custom answer box).
    Custom,
}

/// Live dialog state for the currently shown question.
#[derive(Debug, Clone)]
pub struct QuestionMenu {
    pub prompt: QuestionPrompt,
    /// Selected row over `options` + 1 (the trailing "custom" row).
    pub selected: usize,
    pub custom_input: String,
    pub custom_cursor: usize,
    pub focus: QuestionFocus,
}

/// Outcome of one keystroke.
#[derive(Debug, PartialEq, Eq)]
pub enum QuestionAction {
    Idle,
    /// Submit `(id, answer)` — resolve on the hub and close the dialog.
    Answer(String, String),
    /// Esc — resolve with the skip text and close.
    Skip(String),
}

impl QuestionMenu {
    pub fn new(prompt: QuestionPrompt) -> Self {
        QuestionMenu {
            prompt,
            selected: 0,
            custom_input: String::new(),
            custom_cursor: 0,
            focus: QuestionFocus::Options,
        }
    }

    /// Total selectable rows: preset options plus the trailing custom row.
    pub fn rows(&self) -> usize {
        self.prompt.options.len() + 1
    }

    /// Index of the trailing "✎ custom answer" row.
    pub fn custom_row(&self) -> usize {
        self.prompt.options.len()
    }

    /// The answer Enter would submit right now: custom text wins (priority),
    /// else the selected preset option.
    pub fn current_answer(&self) -> Option<String> {
        let custom = self.custom_input.trim();
        if !custom.is_empty() {
            return Some(custom.to_string());
        }
        if self.focus == QuestionFocus::Custom {
            return None; // empty custom box, custom focused: nothing to say yet
        }
        self.prompt.options.get(self.selected).cloned()
    }

    /// Paste payload goes into the custom box (and focuses it).
    pub fn paste_custom(&mut self, text: &str) {
        for ch in text.chars() {
            insert_at_cursor(&mut self.custom_input, &mut self.custom_cursor, ch);
        }
        self.focus = QuestionFocus::Custom;
    }

    /// Single-line header for the chat transcript: the question itself.
    pub fn header(&self) -> Line<'static> {
        Line::from(self.prompt.question.clone())
    }
}

/// Handle one keystroke. Pure: returns the action, mutates the menu in place.
pub fn handle_question_key(menu: &mut QuestionMenu, k: KeyEvent) -> QuestionAction {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        return QuestionAction::Skip(menu.prompt.id.clone());
    }
    match menu.focus {
        QuestionFocus::Options => handle_options_focus(menu, k),
        QuestionFocus::Custom => handle_custom_focus(menu, k),
    }
}

fn handle_options_focus(menu: &mut QuestionMenu, k: KeyEvent) -> QuestionAction {
    let rows = menu.rows();
    match k.code {
        KeyCode::Esc => QuestionAction::Skip(menu.prompt.id.clone()),
        KeyCode::Up | KeyCode::Char('k') => {
            menu.selected = if menu.selected == 0 { rows - 1 } else { menu.selected - 1 };
            QuestionAction::Idle
        }
        KeyCode::Down | KeyCode::Char('j') => {
            menu.selected = (menu.selected + 1) % rows;
            QuestionAction::Idle
        }
        KeyCode::Tab | KeyCode::BackTab => {
            menu.focus = QuestionFocus::Custom;
            QuestionAction::Idle
        }
        KeyCode::Enter => {
            if menu.selected == menu.custom_row() {
                // The "✎ custom…" row: switch to the free-text box instead of
                // submitting an empty custom answer.
                menu.focus = QuestionFocus::Custom;
                QuestionAction::Idle
            } else {
                submit(menu)
            }
        }
        _ => QuestionAction::Idle,
    }
}

fn handle_custom_focus(menu: &mut QuestionMenu, k: KeyEvent) -> QuestionAction {
    match k.code {
        KeyCode::Esc => QuestionAction::Skip(menu.prompt.id.clone()),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up => {
            menu.focus = QuestionFocus::Options;
            QuestionAction::Idle
        }
        KeyCode::Left => {
            menu.custom_cursor = menu.custom_cursor.saturating_sub(1);
            QuestionAction::Idle
        }
        KeyCode::Right => {
            if menu.custom_cursor < menu.custom_input.chars().count() {
                menu.custom_cursor += 1;
            }
            QuestionAction::Idle
        }
        KeyCode::Backspace => {
            backspace_at(&mut menu.custom_input, &mut menu.custom_cursor);
            QuestionAction::Idle
        }
        KeyCode::Enter => submit(menu),
        KeyCode::Char(c) => {
            insert_at_cursor(&mut menu.custom_input, &mut menu.custom_cursor, c);
            QuestionAction::Idle
        }
        _ => QuestionAction::Idle,
    }
}

/// Custom text has priority; an empty custom box falls back to the selected
/// preset option (when one is selected); otherwise Enter is a no-op.
fn submit(menu: &mut QuestionMenu) -> QuestionAction {
    match menu.current_answer() {
        Some(answer) => QuestionAction::Answer(menu.prompt.id.clone(), answer),
        None => QuestionAction::Idle,
    }
}

fn insert_at_cursor(buf: &mut String, cursor: &mut usize, ch: char) {
    let at = buf
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(buf.len());
    buf.insert(at, ch);
    *cursor += 1;
}

fn backspace_at(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let at = buf
        .char_indices()
        .nth(*cursor - 1)
        .map(|(i, _)| i)
        .unwrap_or(buf.len());
    buf.remove(at);
    *cursor -= 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn menu() -> QuestionMenu {
        QuestionMenu::new(QuestionPrompt {
            id: "q-1".into(),
            question: "Which database?".into(),
            options: vec!["sqlite".into(), "postgres".into()],
        })
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn down_then_enter_answers_the_selected_option() {
        let mut m = menu();
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Down)), QuestionAction::Idle);
        assert_eq!(
            handle_question_key(&mut m, key(KeyCode::Enter)),
            QuestionAction::Answer("q-1".into(), "postgres".into())
        );
    }

    #[test]
    fn up_from_top_wraps_to_the_custom_row() {
        let mut m = menu();
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Up)), QuestionAction::Idle);
        assert_eq!(m.selected, m.custom_row());
    }

    #[test]
    fn enter_on_custom_row_focuses_the_custom_box() {
        let mut m = menu();
        for _ in 0..2 {
            handle_question_key(&mut m, key(KeyCode::Down));
        }
        assert_eq!(m.selected, m.custom_row());
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Enter)), QuestionAction::Idle);
        assert_eq!(m.focus, QuestionFocus::Custom);
    }

    #[test]
    fn typed_custom_text_has_priority_over_the_selected_option() {
        let mut m = menu();
        handle_question_key(&mut m, key(KeyCode::Tab));
        for ch in "mysql 8".chars() {
            handle_question_key(&mut m, key(KeyCode::Char(ch)));
        }
        assert_eq!(
            handle_question_key(&mut m, key(KeyCode::Enter)),
            QuestionAction::Answer("q-1".into(), "mysql 8".into())
        );
    }

    #[test]
    fn empty_custom_box_falls_back_to_the_selected_option() {
        let mut m = menu();
        handle_question_key(&mut m, key(KeyCode::Down)); // select "postgres"
        handle_question_key(&mut m, key(KeyCode::Tab)); // jump into custom box
        // Enter with an empty custom box: current_answer has custom-priority
        // logic — custom empty + custom focus means "nothing to say yet".
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Enter)), QuestionAction::Idle);
        // But from the options list with empty custom text the preset wins.
        handle_question_key(&mut m, key(KeyCode::Tab));
        assert_eq!(
            handle_question_key(&mut m, key(KeyCode::Enter)),
            QuestionAction::Answer("q-1".into(), "postgres".into())
        );
    }

    #[test]
    fn esc_skips_regardless_of_focus() {
        let mut m = menu();
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Esc)), QuestionAction::Skip("q-1".into()));
        handle_question_key(&mut m, key(KeyCode::Tab));
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Esc)), QuestionAction::Skip("q-1".into()));
    }

    #[test]
    fn custom_box_edits_text() {
        let mut m = menu();
        handle_question_key(&mut m, key(KeyCode::Tab));
        for ch in "ab".chars() {
            handle_question_key(&mut m, key(KeyCode::Char(ch)));
        }
        assert_eq!(m.custom_input, "ab");
        handle_question_key(&mut m, key(KeyCode::Left));
        handle_question_key(&mut m, key(KeyCode::Char('X')));
        assert_eq!(m.custom_input, "aXb");
        handle_question_key(&mut m, key(KeyCode::Backspace));
        assert_eq!(m.custom_input, "ab");
    }

    #[test]
    fn paste_fills_the_custom_box_and_focuses_it() {
        let mut m = menu();
        m.paste_custom("pasted answer");
        assert_eq!(m.custom_input, "pasted answer");
        assert_eq!(m.focus, QuestionFocus::Custom);
    }

    #[test]
    fn no_options_still_offers_the_custom_row() {
        let mut m = QuestionMenu::new(QuestionPrompt {
            id: "q-2".into(),
            question: "Free-form?".into(),
            options: vec![],
        });
        assert_eq!(m.rows(), 1);
        // The only row IS the custom row.
        assert_eq!(handle_question_key(&mut m, key(KeyCode::Enter)), QuestionAction::Idle);
        assert_eq!(m.focus, QuestionFocus::Custom);
        handle_question_key(&mut m, key(KeyCode::Char('x')));
        assert_eq!(
            handle_question_key(&mut m, key(KeyCode::Enter)),
            QuestionAction::Answer("q-2".into(), "x".into())
        );
    }
}
