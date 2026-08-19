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
            item.custom_cursor =
                composer::move_cursor_vertical(&item.custom_input, item.custom_cursor, 1, width, 0);
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
            let (text, cursor) = composer::insert_newline(&item.custom_input, item.custom_cursor);
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
    (0..cursor)
        .rev()
        .find(|&i| chars[i] == '\n')
        .map_or(0, |i| i + 1)
}

/// Char index of the end of the logical line holding `cursor` (before '\n').
fn line_end(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    (cursor..chars.len())
        .find(|&i| chars[i] == '\n')
        .unwrap_or(chars.len())
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
#[path = "state_tests.rs"]
mod tests;
