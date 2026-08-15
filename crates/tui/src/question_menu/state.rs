//! Pure state machine for the plan-mode question dialog.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

/// One question parsed from a `question` ToolStart event.
#[derive(Debug, Clone)]
pub struct QuestionPrompt {
    /// Tool-call id used by [`opencoder_session::tools::question::QuestionHub`].
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
}

/// Which part of the active question owns keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionFocus {
    /// Up/down select an answer; left/right change question.
    Options,
    /// Text editing in the always-visible custom input row.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    Answer(String),
    Skip,
}

/// Per-question state. It remains in the dialog after confirmation so the
/// user can revisit and edit it before the complete batch is submitted.
#[derive(Debug, Clone)]
pub struct QuestionItem {
    pub prompt: QuestionPrompt,
    /// Selected row over `prompt.options` plus the trailing Custom option.
    pub selected: usize,
    pub custom_input: String,
    pub custom_cursor: usize,
    decision: Option<Decision>,
}

impl QuestionItem {
    fn new(prompt: QuestionPrompt) -> Self {
        Self {
            prompt,
            selected: 0,
            custom_input: String::new(),
            custom_cursor: 0,
            decision: None,
        }
    }

    pub fn rows(&self) -> usize {
        self.prompt.options.len() + 1
    }

    pub fn custom_row(&self) -> usize {
        self.prompt.options.len()
    }

    pub fn confirmed(&self) -> bool {
        self.decision.is_some()
    }

    /// Compose the tool result for the current selection. Custom-only answers
    /// require text; preset answers append non-empty user text on a new line.
    pub fn current_answer(&self) -> Option<String> {
        let custom = self.custom_input.trim();
        if self.selected == self.custom_row() {
            return (!custom.is_empty()).then(|| custom.to_string());
        }
        self.prompt.options.get(self.selected).map(|option| {
            if custom.is_empty() {
                option.clone()
            } else {
                format!("{option}\n{custom}")
            }
        })
    }

    fn invalidate(&mut self) {
        self.decision = None;
    }
}

/// Complete multi-question dialog state.
#[derive(Debug, Clone)]
pub struct QuestionMenu {
    pub questions: Vec<QuestionItem>,
    pub active: usize,
    pub focus: QuestionFocus,
}

/// One answer held until every question has been confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionResponse {
    pub id: String,
    /// None represents an explicit Esc/Ctrl+D skip.
    pub answer: Option<String>,
}

/// Outcome of one keystroke.
#[derive(Debug, PartialEq, Eq)]
pub enum QuestionAction {
    Idle,
    /// All questions are confirmed; resolve the complete batch together.
    Submit(Vec<QuestionResponse>),
}

impl QuestionMenu {
    pub fn new(prompt: QuestionPrompt) -> Self {
        Self {
            questions: vec![QuestionItem::new(prompt)],
            active: 0,
            focus: QuestionFocus::Options,
        }
    }

    pub fn push(&mut self, prompt: QuestionPrompt) {
        self.questions.push(QuestionItem::new(prompt));
    }

    pub fn current(&self) -> &QuestionItem {
        &self.questions[self.active]
    }

    pub fn current_mut(&mut self) -> &mut QuestionItem {
        &mut self.questions[self.active]
    }

    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.questions.iter().map(|q| q.prompt.id.as_str())
    }

    pub fn remove(&mut self, id: &str) {
        let Some(index) = self.questions.iter().position(|q| q.prompt.id == id) else {
            return;
        };
        self.questions.remove(index);
        if self.questions.is_empty() {
            self.active = 0;
        } else if index < self.active || self.active == self.questions.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn completed_responses(&self) -> Option<Vec<QuestionResponse>> {
        self.questions
            .iter()
            .map(|q| match q.decision.as_ref()? {
                Decision::Answer(answer) => Some(QuestionResponse {
                    id: q.prompt.id.clone(),
                    answer: Some(answer.clone()),
                }),
                Decision::Skip => Some(QuestionResponse {
                    id: q.prompt.id.clone(),
                    answer: None,
                }),
            })
            .collect()
    }

    /// Paste goes to the active question and focuses its custom input.
    pub fn paste_custom(&mut self, text: &str) {
        let text = crate::terminal_text::sanitize_single_line(text);
        let item = self.current_mut();
        item.invalidate();
        for ch in text.chars() {
            insert_at_cursor(&mut item.custom_input, &mut item.custom_cursor, ch);
        }
        self.focus = QuestionFocus::Custom;
    }

    pub fn header(&self) -> Line<'static> {
        Line::from(self.current().prompt.question.clone())
    }

    fn change_question(&mut self, delta: isize) {
        let len = self.questions.len();
        if len > 1 {
            self.active = (self.active as isize + delta).rem_euclid(len as isize) as usize;
        }
    }

    fn advance_to_unconfirmed(&mut self) {
        let len = self.questions.len();
        if let Some(offset) =
            (1..=len).find(|offset| !self.questions[(self.active + offset) % len].confirmed())
        {
            self.active = (self.active + offset) % len;
        }
        self.focus = QuestionFocus::Options;
    }
}

/// Handle one keystroke. Pure: returns an action and mutates only `menu`.
pub fn handle_question_key(menu: &mut QuestionMenu, key: KeyEvent) -> QuestionAction {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        return confirm_skip(menu);
    }
    match menu.focus {
        QuestionFocus::Options => handle_options_focus(menu, key),
        QuestionFocus::Custom => handle_custom_focus(menu, key),
    }
}

fn handle_options_focus(menu: &mut QuestionMenu, key: KeyEvent) -> QuestionAction {
    match key.code {
        KeyCode::Esc => confirm_skip(menu),
        KeyCode::Left => {
            menu.change_question(-1);
            QuestionAction::Idle
        }
        KeyCode::Right => {
            menu.change_question(1);
            QuestionAction::Idle
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let item = menu.current_mut();
            item.invalidate();
            item.selected = if item.selected == 0 {
                item.rows() - 1
            } else {
                item.selected - 1
            };
            QuestionAction::Idle
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let item = menu.current_mut();
            item.invalidate();
            item.selected = (item.selected + 1) % item.rows();
            QuestionAction::Idle
        }
        KeyCode::Tab | KeyCode::BackTab => {
            menu.focus = QuestionFocus::Custom;
            QuestionAction::Idle
        }
        KeyCode::Enter
            if menu.current().selected == menu.current().custom_row()
                && menu.current().current_answer().is_none() =>
        {
            menu.focus = QuestionFocus::Custom;
            QuestionAction::Idle
        }
        KeyCode::Enter => confirm_answer(menu),
        _ => QuestionAction::Idle,
    }
}

fn handle_custom_focus(menu: &mut QuestionMenu, key: KeyEvent) -> QuestionAction {
    match key.code {
        KeyCode::Esc => confirm_skip(menu),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up => {
            menu.focus = QuestionFocus::Options;
            QuestionAction::Idle
        }
        KeyCode::Left => {
            let item = menu.current_mut();
            item.custom_cursor = item.custom_cursor.saturating_sub(1);
            QuestionAction::Idle
        }
        KeyCode::Right => {
            let item = menu.current_mut();
            if item.custom_cursor < item.custom_input.chars().count() {
                item.custom_cursor += 1;
            }
            QuestionAction::Idle
        }
        KeyCode::Backspace => {
            let item = menu.current_mut();
            if backspace_at(&mut item.custom_input, &mut item.custom_cursor) {
                item.invalidate();
            }
            QuestionAction::Idle
        }
        KeyCode::Enter => confirm_answer(menu),
        KeyCode::Char(ch) => {
            let item = menu.current_mut();
            insert_at_cursor(&mut item.custom_input, &mut item.custom_cursor, ch);
            item.invalidate();
            QuestionAction::Idle
        }
        _ => QuestionAction::Idle,
    }
}

fn confirm_answer(menu: &mut QuestionMenu) -> QuestionAction {
    let Some(answer) = menu.current().current_answer() else {
        return QuestionAction::Idle;
    };
    menu.current_mut().decision = Some(Decision::Answer(answer));
    finish_or_advance(menu)
}

fn confirm_skip(menu: &mut QuestionMenu) -> QuestionAction {
    menu.current_mut().decision = Some(Decision::Skip);
    finish_or_advance(menu)
}

fn finish_or_advance(menu: &mut QuestionMenu) -> QuestionAction {
    if let Some(responses) = menu.completed_responses() {
        QuestionAction::Submit(responses)
    } else {
        menu.advance_to_unconfirmed();
        QuestionAction::Idle
    }
}

fn insert_at_cursor(buf: &mut String, cursor: &mut usize, ch: char) {
    let at = buf
        .char_indices()
        .nth(*cursor)
        .map(|(index, _)| index)
        .unwrap_or(buf.len());
    buf.insert(at, ch);
    *cursor += 1;
}

fn backspace_at(buf: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    let at = buf
        .char_indices()
        .nth(*cursor - 1)
        .map(|(index, _)| index)
        .unwrap_or(buf.len());
    buf.remove(at);
    *cursor -= 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(id: &str, question: &str) -> QuestionPrompt {
        QuestionPrompt {
            id: id.into(),
            question: question.into(),
            options: vec!["sqlite".into(), "postgres".into()],
        }
    }

    fn menu() -> QuestionMenu {
        let mut menu = QuestionMenu::new(prompt("q1", "Database?"));
        menu.push(prompt("q2", "Runtime?"));
        menu
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(menu: &mut QuestionMenu, code: KeyCode) -> QuestionAction {
        handle_question_key(menu, key(code))
    }

    #[test]
    fn arrows_switch_questions_and_preserve_each_questions_state() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Down);
        press(&mut menu, KeyCode::Tab);
        for ch in "first".chars() {
            press(&mut menu, KeyCode::Char(ch));
        }
        press(&mut menu, KeyCode::Tab);
        press(&mut menu, KeyCode::Right);
        press(&mut menu, KeyCode::Up);
        press(&mut menu, KeyCode::Tab);
        for ch in "second".chars() {
            press(&mut menu, KeyCode::Char(ch));
        }
        press(&mut menu, KeyCode::Tab);
        press(&mut menu, KeyCode::Left);

        assert_eq!(menu.active, 0);
        assert_eq!(menu.current().selected, 1);
        assert_eq!(menu.current().custom_input, "first");
        assert_eq!(menu.current().custom_cursor, 5);
        assert_eq!(menu.questions[1].selected, 2);
        assert_eq!(menu.questions[1].custom_input, "second");
    }

    #[test]
    fn preset_answer_appends_custom_input_but_custom_option_uses_only_input() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Down);
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("version 16");
        assert_eq!(
            menu.current().current_answer().as_deref(),
            Some("postgres\nversion 16")
        );

        press(&mut menu, KeyCode::Tab);
        press(&mut menu, KeyCode::Down);
        assert_eq!(menu.current().selected, menu.current().custom_row());
        assert_eq!(
            menu.current().current_answer().as_deref(),
            Some("version 16")
        );
    }

    #[test]
    fn confirmations_are_held_until_every_question_is_confirmed() {
        let mut menu = menu();
        assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
        assert_eq!(menu.active, 1);
        assert!(menu.questions[0].confirmed());
        assert!(!menu.questions[1].confirmed());

        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            QuestionAction::Submit(vec![
                QuestionResponse {
                    id: "q1".into(),
                    answer: Some("sqlite".into())
                },
                QuestionResponse {
                    id: "q2".into(),
                    answer: Some("sqlite".into())
                },
            ])
        );
    }

    #[test]
    fn editing_a_confirmed_question_requires_reconfirmation() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Enter);
        press(&mut menu, KeyCode::Left);
        press(&mut menu, KeyCode::Down);
        assert!(!menu.questions[0].confirmed());
        press(&mut menu, KeyCode::Right);
        assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
        assert_eq!(menu.active, 0);
    }

    #[test]
    fn custom_option_requires_text_and_enter_focuses_the_input() {
        let mut menu = QuestionMenu::new(QuestionPrompt {
            id: "q1".into(),
            question: "Free form?".into(),
            options: vec![],
        });
        assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
        assert_eq!(menu.focus, QuestionFocus::Custom);
        assert_eq!(press(&mut menu, KeyCode::Enter), QuestionAction::Idle);
        press(&mut menu, KeyCode::Char('中'));
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            QuestionAction::Submit(vec![QuestionResponse {
                id: "q1".into(),
                answer: Some("中".into()),
            }])
        );
    }

    #[test]
    fn custom_cursor_edits_unicode_by_character_index() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("你好a");
        press(&mut menu, KeyCode::Left);
        press(&mut menu, KeyCode::Left);
        press(&mut menu, KeyCode::Char('X'));
        assert_eq!(menu.current().custom_input, "你X好a");
        press(&mut menu, KeyCode::Backspace);
        assert_eq!(menu.current().custom_input, "你好a");
        assert_eq!(menu.current().custom_cursor, 1);
    }

    #[test]
    fn paste_keeps_the_custom_input_single_line_and_cursor_aligned() {
        let mut menu = menu();
        menu.paste_custom("first\nsecond\tpart");
        assert_eq!(menu.current().custom_input, "first second    part");
        assert_eq!(
            menu.current().custom_cursor,
            menu.current().custom_input.chars().count()
        );
    }

    #[test]
    fn skip_is_batched_with_answers() {
        let mut menu = menu();
        assert_eq!(press(&mut menu, KeyCode::Esc), QuestionAction::Idle);
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            QuestionAction::Submit(vec![
                QuestionResponse {
                    id: "q1".into(),
                    answer: None
                },
                QuestionResponse {
                    id: "q2".into(),
                    answer: Some("sqlite".into())
                },
            ])
        );
    }
}
