//! Pure state machine for the plan-mode question dialog.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

use crate::composer;

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
    /// Multi-line paste is preserved; the shared composer boundary strips
    /// terminal-corrupting controls and enforces the size cap.
    pub fn paste_custom(&mut self, text: &str) {
        let item = self.current_mut();
        item.invalidate();
        let (input, cursor) = composer::insert_str(&item.custom_input, item.custom_cursor, text);
        item.custom_input = input;
        item.custom_cursor = cursor;
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
/// `width` is the custom input's wrap width ([`super::input_wrap_width`]) so
/// vertical cursor movement mirrors the renderer's soft-wrapped rows.
pub fn handle_question_key(menu: &mut QuestionMenu, key: KeyEvent, width: u16) -> QuestionAction {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        return confirm_skip(menu);
    }
    match menu.focus {
        QuestionFocus::Options => handle_options_focus(menu, key),
        QuestionFocus::Custom => handle_custom_focus(menu, key, width),
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

fn handle_custom_focus(menu: &mut QuestionMenu, key: KeyEvent, width: u16) -> QuestionAction {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl && !alt {
        return custom_ctrl_key(menu, key.code);
    }
    if alt && !ctrl {
        return custom_alt_key(menu, key.code);
    }
    match key.code {
        KeyCode::Esc => confirm_skip(menu),
        KeyCode::Tab | KeyCode::BackTab => {
            menu.focus = QuestionFocus::Options;
            QuestionAction::Idle
        }
        KeyCode::Up => {
            // Above the first visual row the key moves the cursor between
            // wrapped rows; only there does it hand focus back to Options.
            let item = menu.current();
            let (row, _) =
                composer::cursor_row_col(&item.custom_input, item.custom_cursor, width, 0);
            if row > 0 {
                let item = menu.current_mut();
                item.custom_cursor = composer::move_cursor_vertical(
                    &item.custom_input,
                    item.custom_cursor,
                    -1,
                    width,
                    0,
                );
            } else {
                menu.focus = QuestionFocus::Options;
            }
            QuestionAction::Idle
        }
        KeyCode::Down => {
            let item = menu.current_mut();
            item.custom_cursor = composer::move_cursor_vertical(
                &item.custom_input,
                item.custom_cursor,
                1,
                width,
                0,
            );
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
        KeyCode::Home => {
            let item = menu.current_mut();
            item.custom_cursor = line_start(&item.custom_input, item.custom_cursor);
            QuestionAction::Idle
        }
        KeyCode::End => {
            let item = menu.current_mut();
            item.custom_cursor = line_end(&item.custom_input, item.custom_cursor);
            QuestionAction::Idle
        }
        KeyCode::Backspace => {
            let item = menu.current_mut();
            if let Some((text, cursor)) =
                composer::backspace(&item.custom_input, item.custom_cursor)
            {
                item.custom_input = text;
                item.custom_cursor = cursor;
                item.invalidate();
            }
            QuestionAction::Idle
        }
        KeyCode::Delete => {
            let item = menu.current_mut();
            if let Some((text, cursor)) = delete_forward(&item.custom_input, item.custom_cursor) {
                item.custom_input = text;
                item.custom_cursor = cursor;
                item.invalidate();
            }
            QuestionAction::Idle
        }
        // Enter confirms; Shift+Enter inserts an explicit newline instead.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            let item = menu.current_mut();
            let (text, cursor) =
                composer::insert_newline(&item.custom_input, item.custom_cursor);
            item.custom_input = text;
            item.custom_cursor = cursor;
            item.invalidate();
            QuestionAction::Idle
        }
        KeyCode::Enter => confirm_answer(menu),
        KeyCode::Char(ch) => {
            let item = menu.current_mut();
            let (text, cursor) = composer::insert_char(&item.custom_input, item.custom_cursor, ch);
            item.custom_input = text;
            item.custom_cursor = cursor;
            item.invalidate();
            QuestionAction::Idle
        }
        _ => QuestionAction::Idle,
    }
}

/// Readline-style Ctrl bindings inside the custom input.
fn custom_ctrl_key(menu: &mut QuestionMenu, code: KeyCode) -> QuestionAction {
    let item = menu.current_mut();
    match code {
        KeyCode::Char('a' | 'A') => {
            item.custom_cursor = line_start(&item.custom_input, item.custom_cursor);
        }
        KeyCode::Char('e' | 'E') => {
            item.custom_cursor = line_end(&item.custom_input, item.custom_cursor);
        }
        KeyCode::Char('u' | 'U') => {
            if !item.custom_input.is_empty() {
                item.custom_input.clear();
                item.custom_cursor = 0;
                item.invalidate();
            }
        }
        KeyCode::Char('k' | 'K') => {
            if item.custom_cursor < item.custom_input.chars().count() {
                item.custom_input = item.custom_input.chars().take(item.custom_cursor).collect();
                item.invalidate();
            }
        }
        KeyCode::Char('w' | 'W') => {
            if let Some((text, cursor)) =
                composer::delete_word_back(&item.custom_input, item.custom_cursor)
            {
                item.custom_input = text;
                item.custom_cursor = cursor;
                item.invalidate();
            }
        }
        KeyCode::Char('j' | 'J') => {
            let (text, cursor) = composer::insert_newline(&item.custom_input, item.custom_cursor);
            item.custom_input = text;
            item.custom_cursor = cursor;
            item.invalidate();
        }
        // Remaining Ctrl combos never reach the text buffer.
        _ => {}
    }
    QuestionAction::Idle
}

/// Alt bindings: word motion plus Alt+Backspace / Alt+Enter. Unhandled Alt
/// combos are swallowed so tmux Esc-merge garbage cannot reach the buffer.
fn custom_alt_key(menu: &mut QuestionMenu, code: KeyCode) -> QuestionAction {
    let item = menu.current_mut();
    match code {
        KeyCode::Backspace => {
            if let Some((text, cursor)) =
                composer::delete_word_back(&item.custom_input, item.custom_cursor)
            {
                item.custom_input = text;
                item.custom_cursor = cursor;
                item.invalidate();
            }
        }
        KeyCode::Char('b' | 'B') => {
            item.custom_cursor = composer::backward_word(&item.custom_input, item.custom_cursor);
        }
        KeyCode::Char('f' | 'F') => {
            item.custom_cursor = composer::forward_word(&item.custom_input, item.custom_cursor);
        }
        KeyCode::Enter => {
            let (text, cursor) = composer::insert_newline(&item.custom_input, item.custom_cursor);
            item.custom_input = text;
            item.custom_cursor = cursor;
            item.invalidate();
        }
        _ => {}
    }
    QuestionAction::Idle
}

/// Char index of the start of the logical line holding `cursor`.
fn line_start(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    (0..cursor).rev().find(|&i| chars[i] == '\n').map_or(0, |i| i + 1)
}

/// Char index of the end of the logical line holding `cursor` (before '\n').
fn line_end(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    (cursor..chars.len()).find(|&i| chars[i] == '\n').unwrap_or(chars.len())
}

/// Delete the char under the cursor (forward delete).
fn delete_forward(text: &str, cursor: usize) -> Option<(String, usize)> {
    if cursor >= text.chars().count() {
        return None;
    }
    let mut out: String = text.chars().take(cursor).collect();
    out.extend(text.chars().skip(cursor + 1));
    Some((out, cursor))
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

    /// Terminal-wide (80 col) question popup wrap width.
    const WIDTH: u16 = 55;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(menu: &mut QuestionMenu, code: KeyCode) -> QuestionAction {
        handle_question_key(menu, key(code), WIDTH)
    }

    fn press_with(menu: &mut QuestionMenu, event: KeyEvent, width: u16) -> QuestionAction {
        handle_question_key(menu, event, width)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn alt(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
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
    fn paste_preserves_newlines_and_aligns_the_cursor() {
        let mut menu = menu();
        menu.paste_custom("first\nsecond\tpart");
        assert_eq!(menu.current().custom_input, "first\nsecond    part");
        assert_eq!(
            menu.current().custom_cursor,
            menu.current().custom_input.chars().count()
        );
    }

    #[test]
    fn readline_jump_keys_reach_line_boundaries() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("one two\nthree");
        // Jump keys operate on the logical line, not the whole buffer.
        press_with(&mut menu, ctrl('a'), WIDTH);
        assert_eq!(menu.current().custom_cursor, 8); // start of "three"
        press(&mut menu, KeyCode::Home);
        assert_eq!(menu.current().custom_cursor, 8);
        press_with(&mut menu, ctrl('e'), WIDTH);
        assert_eq!(menu.current().custom_cursor, 13);
        press(&mut menu, KeyCode::End);
        assert_eq!(menu.current().custom_cursor, 13);
        for _ in 0..6 {
            press(&mut menu, KeyCode::Left); // onto the first line
        }
        press(&mut menu, KeyCode::Home);
        assert_eq!(menu.current().custom_cursor, 0);
        press_with(&mut menu, ctrl('e'), WIDTH);
        assert_eq!(menu.current().custom_cursor, 7);
    }

    #[test]
    fn ctrl_u_clears_and_ctrl_k_deletes_to_the_end() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("hello world");
        for _ in 0..6 {
            press(&mut menu, KeyCode::Left);
        }
        assert_eq!(menu.current().custom_cursor, 5);
        press_with(&mut menu, ctrl('k'), WIDTH);
        assert_eq!(menu.current().custom_input, "hello");
        assert_eq!(menu.current().custom_cursor, 5);
        press_with(&mut menu, ctrl('u'), WIDTH);
        assert_eq!(menu.current().custom_input, "");
        assert_eq!(menu.current().custom_cursor, 0);
    }

    #[test]
    fn word_keys_delete_and_move_by_word() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("foo bar baz");
        for _ in 0..3 {
            press(&mut menu, KeyCode::Left);
        }
        assert_eq!(menu.current().custom_cursor, 8);
        press_with(&mut menu, ctrl('w'), WIDTH);
        assert_eq!(menu.current().custom_input, "foo baz");
        assert_eq!(menu.current().custom_cursor, 4);
        press_with(&mut menu, alt('f'), WIDTH);
        assert_eq!(menu.current().custom_cursor, 7); // end of "baz"
        press_with(&mut menu, alt('b'), WIDTH);
        assert_eq!(menu.current().custom_cursor, 4);
        press_with(
            &mut menu,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
            WIDTH,
        );
        assert_eq!(menu.current().custom_input, "baz");
        assert_eq!(menu.current().custom_cursor, 0);
    }

    #[test]
    fn delete_key_removes_the_char_under_the_cursor() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("abc");
        press(&mut menu, KeyCode::Home);
        press(&mut menu, KeyCode::Delete);
        assert_eq!(menu.current().custom_input, "bc");
        assert_eq!(menu.current().custom_cursor, 0);
        press(&mut menu, KeyCode::Delete);
        press(&mut menu, KeyCode::Delete);
        assert_eq!(menu.current().custom_input, "");
    }

    #[test]
    fn explicit_newline_keys_keep_enter_as_confirm() {
        let mut menu = QuestionMenu::new(prompt("q1", "Free form?"));
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("line1");
        press_with(
            &mut menu,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            WIDTH,
        );
        press_with(&mut menu, ctrl('j'), WIDTH);
        press(&mut menu, KeyCode::Char('x'));
        assert_eq!(menu.current().custom_input, "line1\n\nx");
        assert_eq!(
            press(&mut menu, KeyCode::Enter),
            QuestionAction::Submit(vec![QuestionResponse {
                id: "q1".into(),
                // Preset option selected: custom details append on a new line.
                answer: Some("sqlite\nline1\n\nx".into()),
            }])
        );
    }

    #[test]
    fn up_down_move_across_wrapped_rows_before_leaving_the_input() {
        let mut menu = menu();
        press_with(&mut menu, key(KeyCode::Tab), WIDTH);
        menu.paste_custom("aaaaaaaaaaaa"); // wraps to 2 rows at width 10
        assert_eq!(menu.focus, QuestionFocus::Custom);
        press_with(&mut menu, key(KeyCode::Up), 10);
        assert_eq!(menu.current().custom_cursor, 2);
        assert_eq!(menu.focus, QuestionFocus::Custom);
        press_with(&mut menu, key(KeyCode::Up), 10);
        assert_eq!(menu.focus, QuestionFocus::Options);
        press_with(&mut menu, key(KeyCode::Tab), WIDTH);
        press_with(&mut menu, key(KeyCode::Down), 10);
        assert_eq!(menu.current().custom_cursor, 12);
        // Down on the last visual row is a no-op that keeps the focus.
        press_with(&mut menu, key(KeyCode::Down), 10);
        assert_eq!(menu.current().custom_cursor, 12);
        assert_eq!(menu.focus, QuestionFocus::Custom);
    }

    #[test]
    fn up_crosses_explicit_newlines_too() {
        let mut menu = menu();
        press(&mut menu, KeyCode::Tab);
        menu.paste_custom("one\ntwo");
        press(&mut menu, KeyCode::Up);
        assert_eq!(menu.current().custom_cursor, 3);
        assert_eq!(menu.focus, QuestionFocus::Custom);
        press(&mut menu, KeyCode::Up);
        assert_eq!(menu.focus, QuestionFocus::Options);
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
