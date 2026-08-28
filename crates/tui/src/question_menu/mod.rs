//! Multi-question dialog for primary-agent `question` tool calls.
//!
//! All answers are held in the pure state machine until every visible
//! question is confirmed, then resolved directly on the shared QuestionHub.

pub mod state;
pub mod view;

use crossterm::event::KeyEvent;
use opencoder_session::tools::question::QuestionHub;
use serde_json::Value;

pub use state::{
    QuestionAction, QuestionFocus, QuestionItem, QuestionMenu, QuestionPrompt, QuestionResponse,
};
pub use view::render_question_popup;

/// Answer sent to the model for an explicitly skipped question.
pub const SKIP_ANSWER: &str = "User skipped the question. Proceed with your best judgment.";

pub fn dialog_state() -> Option<QuestionMenu> {
    None
}

/// Soft-wrap width of the custom input row. Key routing and rendering share
/// this one formula so vertical cursor movement can never drift from the
/// drawn (wrapped) rows: popup width -> popup inner -> input border (2) ->
/// one reserved cell for the hardware cursor.
pub fn input_wrap_width(area_width: u16) -> u16 {
    let popup_width = 60u16.min(area_width.saturating_sub(4));
    popup_width.saturating_sub(5).max(1)
}

/// Parse a question ToolStart payload into a dialog prompt.
pub fn prompt_from_input(id: &str, input: &Value) -> Option<QuestionPrompt> {
    let question = input.get("question")?.as_str()?.trim();
    if question.is_empty() {
        return None;
    }
    let options = input
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(QuestionPrompt {
        id: id.to_string(),
        question: question.to_string(),
        options,
    })
}

/// Add every parallel ToolStart to one navigable dialog.
pub fn on_tool_start(menu: &mut Option<QuestionMenu>, id: &str, input: &Value) {
    let Some(prompt) = prompt_from_input(id, input) else {
        return;
    };
    match menu {
        Some(menu) => menu.push(prompt),
        None => *menu = Some(QuestionMenu::new(prompt)),
    }
}

/// Drop a question whose tool ended externally (normally a cancel race).
/// If the removed question was the only unfinished one, submit the already
/// confirmed remainder so no still-waiting tool is stranded.
pub fn on_tool_end(menu: &mut Option<QuestionMenu>, id: &str, hub: &QuestionHub) {
    let Some(open) = menu.as_mut() else { return };
    if !open.ids().any(|open_id| open_id == id) {
        return;
    }
    hub.abandon(id);
    open.remove(id);
    if open.is_empty() {
        *menu = None;
        return;
    }
    if let Some(responses) = open.completed_responses() {
        resolve_batch(hub, responses);
        *menu = None;
    }
}

/// Apply a dialog key and resolve only when the full batch is confirmed.
/// `width` is the custom input wrap width (see [`input_wrap_width`]).
pub fn route_question_key(
    menu: &mut Option<QuestionMenu>,
    key: KeyEvent,
    hub: &QuestionHub,
    width: u16,
) {
    let Some(open) = menu.as_mut() else { return };
    if let QuestionAction::Submit(responses) = state::handle_question_key(open, key, width) {
        resolve_batch(hub, responses);
        *menu = None;
    }
}

/// Abandon every question when changing sessions or otherwise closing the
/// owning runtime. This never parks early answers.
pub fn abandon_dialog(menu: &mut Option<QuestionMenu>, hub: &QuestionHub) {
    if let Some(open) = menu.take() {
        for id in open.ids() {
            hub.abandon(id);
        }
    }
}

fn resolve_batch(hub: &QuestionHub, responses: Vec<QuestionResponse>) {
    for response in responses {
        let answer = response.answer.unwrap_or_else(|| SKIP_ANSWER.to_string());
        let _ = hub.resolve(&response.id, answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_session::tools::question::AskOutcome;

    fn input(question: &str, options: &[&str]) -> Value {
        serde_json::json!({ "question": question, "options": options })
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn early_answer(hub: &QuestionHub, id: &str) -> Option<String> {
        match hub.ask(id) {
            AskOutcome::Answered(answer) => Some(answer),
            AskOutcome::Pending(_) => None,
        }
    }

    #[test]
    fn prompt_from_input_parses_question_and_options() {
        let prompt = prompt_from_input("q1", &input("which?", &["a", "b"])).unwrap();
        assert_eq!(prompt.id, "q1");
        assert_eq!(prompt.question, "which?");
        assert_eq!(prompt.options, vec!["a", "b"]);
    }

    #[test]
    fn prompt_from_input_rejects_empty_or_missing_question() {
        assert!(prompt_from_input("q1", &input("   ", &[])).is_none());
        assert!(prompt_from_input("q1", &serde_json::json!({ "options": [] })).is_none());
    }

    #[test]
    fn tool_starts_join_one_navigable_dialog() {
        let mut menu = dialog_state();
        on_tool_start(&mut menu, "q1", &input("first?", &["a"]));
        on_tool_start(&mut menu, "q2", &input("second?", &["b"]));
        let menu = menu.unwrap();
        assert_eq!(menu.len(), 2);
        assert_eq!(menu.questions[0].prompt.id, "q1");
        assert_eq!(menu.questions[1].prompt.id, "q2");
    }

    #[test]
    fn no_question_resolves_until_the_complete_batch_is_confirmed() {
        let hub = QuestionHub::new();
        let mut menu = dialog_state();
        on_tool_start(&mut menu, "q1", &input("first?", &["a"]));
        on_tool_start(&mut menu, "q2", &input("second?", &["b"]));

        route_question_key(&mut menu, key(KeyCode::Enter), &hub, 55);
        assert!(menu.is_some(), "first confirmation keeps the dialog open");
        assert_eq!(
            hub.waiting_count(),
            0,
            "no answer was parked or delivered yet"
        );

        route_question_key(&mut menu, key(KeyCode::Enter), &hub, 55);
        assert!(menu.is_none(), "last confirmation closes the dialog");
        assert_eq!(early_answer(&hub, "q1").as_deref(), Some("a"));
        assert_eq!(early_answer(&hub, "q2").as_deref(), Some("b"));
    }

    #[test]
    fn skipped_question_is_resolved_with_the_batch() {
        let hub = QuestionHub::new();
        let mut menu = dialog_state();
        on_tool_start(&mut menu, "q1", &input("first?", &["a"]));
        on_tool_start(&mut menu, "q2", &input("second?", &["b"]));
        route_question_key(&mut menu, key(KeyCode::Esc), &hub, 55);
        route_question_key(&mut menu, key(KeyCode::Enter), &hub, 55);
        assert_eq!(early_answer(&hub, "q1").as_deref(), Some(SKIP_ANSWER));
        assert_eq!(early_answer(&hub, "q2").as_deref(), Some("b"));
    }

    #[test]
    fn tool_end_removes_only_its_question() {
        let hub = QuestionHub::new();
        let mut menu = dialog_state();
        on_tool_start(&mut menu, "q1", &input("first?", &["a"]));
        on_tool_start(&mut menu, "q2", &input("second?", &["b"]));
        on_tool_end(&mut menu, "q2", &hub);
        let menu = menu.unwrap();
        assert_eq!(menu.len(), 1);
        assert_eq!(menu.current().prompt.id, "q1");
    }

    #[test]
    fn abandon_dialog_clears_all_waiters_without_early_answers() {
        let hub = QuestionHub::new();
        let _first = match hub.ask("q1") {
            AskOutcome::Pending(receiver) => receiver,
            AskOutcome::Answered(_) => panic!("unexpected early answer"),
        };
        let _second = match hub.ask("q2") {
            AskOutcome::Pending(receiver) => receiver,
            AskOutcome::Answered(_) => panic!("unexpected early answer"),
        };
        let mut menu = dialog_state();
        on_tool_start(&mut menu, "q1", &input("first?", &[]));
        on_tool_start(&mut menu, "q2", &input("second?", &[]));
        abandon_dialog(&mut menu, &hub);
        assert!(menu.is_none());
        assert_eq!(hub.waiting_count(), 0);
    }
}
