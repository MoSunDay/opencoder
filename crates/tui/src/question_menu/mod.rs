//! `question` tool dialog: when the plan agent asks the user a clarifying
//! question (ToolStart with `name == "question"`), a compact popup anchored
//! above the composer collects the answer and resolves it directly on the
//! session's shared [`QuestionHub`] — mid-turn, without queuing a `UiCmd`
//! (which would deadlock behind the running prompt).

pub mod state;
pub mod view;

use std::collections::VecDeque;

use crossterm::event::KeyEvent;
use opencoder_session::tools::question::QuestionHub;
use serde_json::Value;

pub use state::{QuestionAction, QuestionFocus, QuestionMenu, QuestionPrompt};
pub use view::render_question_popup;

/// Answer sent to the model when the user skips the dialog (Esc).
pub const SKIP_ANSWER: &str = "User skipped the question. Proceed with your best judgment.";

/// Fresh dialog state for the app loop: `(open menu, queued prompts)`.
pub fn dialog_state() -> (Option<QuestionMenu>, VecDeque<QuestionPrompt>) {
    (None, VecDeque::new())
}

/// Parse a question ToolStart payload into a dialog prompt. Returns None when
/// the payload carries no usable question text (dialog skipped; the tool
/// itself will still wait unless nothing ever resolves).
pub fn prompt_from_input(id: &str, input: &Value) -> Option<QuestionPrompt> {
    let question = input.get("question")?.as_str()?.trim();
    if question.is_empty() {
        return None;
    }
    let options = input
        .get("options")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(QuestionPrompt {
        id: id.to_string(),
        question: question.to_string(),
        options,
    })
}

/// ToolStart(name == "question") → open the dialog, or queue it if one is
/// already showing (the model may emit several parallel questions despite the
/// "one per turn" guidance).
pub fn on_tool_start(
    menu: &mut Option<QuestionMenu>,
    queue: &mut VecDeque<QuestionPrompt>,
    id: &str,
    input: &Value,
) {
    let Some(prompt) = prompt_from_input(id, input) else {
        return;
    };
    if menu.is_none() {
        *menu = Some(QuestionMenu::new(prompt));
    } else {
        queue.push_back(prompt);
    }
}

/// ToolEnd for a question: close the dialog if it is the one showing (cancel
/// path — an answered question has already advanced), and drop it from the
/// queue if it was still waiting.
pub fn on_tool_end(
    menu: &mut Option<QuestionMenu>,
    queue: &mut VecDeque<QuestionPrompt>,
    id: &str,
    hub: &QuestionHub,
) {
    let was_current = menu.as_ref().map(|m| m.prompt.id == id).unwrap_or(false);
    queue.retain(|p| p.id != id);
    if was_current {
        // Belt-and-braces: a cancelled tool future drops its own guard, but
        // clearing here covers any missed race without parking an early answer.
        hub.abandon(id);
        advance(menu, queue);
    }
}

/// Close the current dialog and show the next queued question, if any.
fn advance(menu: &mut Option<QuestionMenu>, queue: &mut VecDeque<QuestionPrompt>) {
    *menu = queue.pop_front().map(QuestionMenu::new);
}

/// Key routing while the dialog is open: apply the keystroke, resolve
/// Answer/Skip actions on the hub, advance to any queued question.
pub fn route_question_key(
    menu: &mut Option<QuestionMenu>,
    queue: &mut VecDeque<QuestionPrompt>,
    k: KeyEvent,
    hub: &QuestionHub,
) {
    let Some(m) = menu.as_mut() else { return };
    match state::handle_question_key(m, k) {
        QuestionAction::Answer(id, answer) => {
            let _ = hub.resolve(&id, answer);
            advance(menu, queue);
        }
        QuestionAction::Skip(id) => {
            let _ = hub.resolve(&id, SKIP_ANSWER.to_string());
            advance(menu, queue);
        }
        QuestionAction::Idle => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_session::tools::question::AskOutcome;

    fn input(question: &str, options: &[&str]) -> Value {
        serde_json::json!({ "question": question, "options": options })
    }

    #[test]
    fn prompt_from_input_parses_question_and_options() {
        let p = prompt_from_input("q1", &input("which?", &["a", "b"])).unwrap();
        assert_eq!(p.id, "q1");
        assert_eq!(p.question, "which?");
        assert_eq!(p.options, vec!["a", "b"]);
    }

    #[test]
    fn prompt_from_input_rejects_empty_or_missing_question() {
        assert!(prompt_from_input("q1", &input("   ", &[])).is_none());
        assert!(prompt_from_input("q1", &serde_json::json!({ "options": [] })).is_none());
    }

    #[test]
    fn tool_start_opens_then_queues_parallel_questions() {
        let mut menu = None;
        let mut queue = VecDeque::new();
        on_tool_start(&mut menu, &mut queue, "q1", &input("first?", &[]));
        assert!(menu.is_some());
        on_tool_start(&mut menu, &mut queue, "q2", &input("second?", &[]));
        assert_eq!(queue.len(), 1);
        assert_eq!(menu.as_ref().unwrap().prompt.id, "q1");
    }

    #[test]
    fn tool_end_closes_only_the_matching_dialog() {
        let hub = QuestionHub::new();
        let mut menu = None;
        let mut queue = VecDeque::new();
        on_tool_start(&mut menu, &mut queue, "q1", &input("first?", &[]));
        on_tool_start(&mut menu, &mut queue, "q2", &input("second?", &[]));
        // ToolEnd for a queued question: it is dropped, the dialog stays.
        on_tool_end(&mut menu, &mut queue, "q2", &hub);
        assert_eq!(menu.as_ref().unwrap().prompt.id, "q1");
        assert!(queue.is_empty());
        // ToolEnd for the showing question (cancel path): dialog closes.
        on_tool_end(&mut menu, &mut queue, "q1", &hub);
        assert!(menu.is_none());
    }

    #[test]
    fn skip_resolves_on_the_hub_and_advances_the_queue() {
        let hub = QuestionHub::new();
        hub.attach();
        let mut menu = None;
        let mut queue = VecDeque::new();
        on_tool_start(&mut menu, &mut queue, "q1", &input("first?", &[]));
        on_tool_start(&mut menu, &mut queue, "q2", &input("second?", &[]));
        route_question_key(
            &mut menu,
            &mut queue,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
            &hub,
        );
        assert_eq!(
            menu.as_ref().unwrap().prompt.id,
            "q2",
            "queued question now showing"
        );
        // The skipped tool call gets its skip answer.
        match hub.ask("q1") {
            AskOutcome::Answered(a) => assert_eq!(a, SKIP_ANSWER),
            _ => panic!("expected Answered"),
        }
    }
}
