//! `question` — structured clarification channel between the model and a
//! human user. The model calls the tool when a requirement is genuinely
//! ambiguous; an attached interactive frontend (TUI) opens a dialog, and the
//! chosen answer is fed back as the tool result *in the same turn*, so the
//! model continues with the answer in context instead of guessing.
//!
//! Answer transport is a shared [`QuestionHub`] (`Arc`), NOT a `UiCmd`:
//! `process_cmd(UiCmd::Prompt)` awaits the whole turn, so an answer queued
//! behind it would deadlock. The hub is a plain id -> oneshot map the UI
//! resolves directly; with no attached listener the tool falls back to a
//! fixed "decide yourself" message so headless runs never hang.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::sync::oneshot;

/// Fixed tool result when no interactive listener is attached (headless
/// `opencode run`, web sessions). The model must proceed on its own judgment.
pub const NO_LISTENER_REPLY: &str = "No interactive user is attached to this session. \
     Proceed with your best judgment and state the assumption in the plan.";

/// Fixed tool result when the answer channel closed without a value (listener
/// vanished mid-question) — treated the same as an explicit user skip.
pub const SKIPPED_REPLY: &str = "User skipped the question. Proceed with your best judgment.";

#[derive(Debug, Default)]
struct HubState {
    attached: bool,
    /// Tool calls currently waiting for an answer, keyed by tool-call id.
    waiting: HashMap<String, oneshot::Sender<String>>,
    /// Answers that arrived before the tool registered (`ToolStart` is
    /// emitted before execution starts, so the UI may resolve early).
    early: HashMap<String, String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shared rendezvous between the question tool (producer side, inside the
/// session runner) and an interactive frontend (consumer side, e.g. the TUI
/// app loop). Pure synchronization state: no I/O, no rendering.
#[derive(Debug, Default)]
pub struct QuestionHub {
    state: Mutex<HubState>,
}

/// Result of registering the tool side of a question.
pub enum AskOutcome {
    /// An answer had already arrived before `ask` (early resolve).
    Answered(String),
    /// Registered; the answer arrives on the receiver.
    Pending(oneshot::Receiver<String>),
}

impl QuestionHub {
    /// Create a fresh shared hub.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Mark an interactive listener as present. Only attached hubs make the
    /// tool wait for a human answer; everything else gets the fallback text.
    pub fn attach(&self) {
        lock(&self.state).attached = true;
    }

    /// Whether an interactive listener has attached.
    pub fn is_attached(&self) -> bool {
        lock(&self.state).attached
    }

    /// Deliver `answer` for `id`. Order-safe both ways: if the tool is
    /// already waiting the oneshot fires; otherwise the answer is parked for
    /// the imminent `ask` (ids are unique per tool call, so a parked answer
    /// can never be mis-delivered). Returns true when the answer was
    /// delivered or parked.
    pub fn resolve(&self, id: &str, answer: String) -> bool {
        let mut st = lock(&self.state);
        if let Some(tx) = st.waiting.remove(id) {
            return tx.send(answer).is_ok();
        }
        st.early.insert(id.to_string(), answer);
        true
    }

    /// Forget a pending question WITHOUT answering it. Dropping the sender
    /// makes the tool-side receiver resolve `Err` (skipped). Never parks an
    /// early answer — unlike [`QuestionHub::resolve`] this only touches
    /// `waiting`, so a cancelled tool call leaves no residue.
    pub fn abandon(&self, id: &str) {
        lock(&self.state).waiting.remove(id);
    }

    /// Register the tool side. Callers must pair this with
    /// [`QuestionHub::abandon`] (via a drop guard) so a cancelled future
    /// removes its sender instead of leaking it in the map.
    pub fn ask(&self, id: &str) -> AskOutcome {
        let mut st = lock(&self.state);
        if let Some(a) = st.early.remove(id) {
            return AskOutcome::Answered(a);
        }
        let (tx, rx) = oneshot::channel();
        st.waiting.insert(id.to_string(), tx);
        AskOutcome::Pending(rx)
    }

    /// Number of tool calls still waiting for a human answer
    /// (diagnostics + tests).
    pub fn waiting_count(&self) -> usize {
        lock(&self.state).waiting.len()
    }
}

/// Removes the hub registration when the tool future is cancelled (the
/// cancel race in `execute_call` drops the losing future). On normal
/// completion the entry is already gone, so `Drop` is a no-op.
struct AskGuard<'a> {
    hub: &'a QuestionHub,
    id: String,
}

impl Drop for AskGuard<'_> {
    fn drop(&mut self) {
        self.hub.abandon(&self.id);
    }
}

/// The question tool. Registered in every session registry; only agents whose
/// `ToolFilter` allows `question` (plan) see its schema.
pub struct QuestionTool {
    hub: Arc<QuestionHub>,
}

impl QuestionTool {
    pub fn new(hub: Arc<QuestionHub>) -> Self {
        Self { hub }
    }
}

#[async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question. Use ONLY when the answer would change the plan \
         and the requirement is genuinely ambiguous; you may ask several in one turn (one per call)."
    }

    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "question".into(),
            json::prop_str("The single question, phrased in one sentence."),
        );
        props.insert(
            "options".into(),
            json::prop_array_str("Up to 4 short suggested answers (optional)."),
        );
        json::object_schema(Value::Object(props), &["question"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let question = input
            .get("question")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let question = match question {
            Some(q) => q,
            None => return Ok(ToolOutput::err("missing required parameter: question")),
        };
        // Headless (run/web): no dialog exists — answer immediately so the
        // turn never blocks on a human that is not there.
        if !self.hub.is_attached() {
            return Ok(ToolOutput::ok(NO_LISTENER_REPLY));
        }
        let _ = question;
        let id = ctx.message_id.clone();
        let _guard = AskGuard {
            hub: self.hub.as_ref(),
            id: id.clone(),
        };
        match self.hub.ask(&id) {
            AskOutcome::Answered(answer) => Ok(ToolOutput::ok(answer)),
            AskOutcome::Pending(rx) => match rx.await {
                Ok(answer) => Ok(ToolOutput::ok(answer)),
                Err(_) => Ok(ToolOutput::ok(SKIPPED_REPLY)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: &str) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            message_id: id.into(),
            agent: "plan".into(),
            working_dir: std::env::temp_dir(),
            max_output: 4096,
            proxy: None,
        }
    }

    fn input(question: &str) -> Value {
        serde_json::json!({ "question": question, "options": ["yes", "no"] })
    }

    #[test]
    fn ask_then_resolve_delivers_the_answer() {
        let hub = QuestionHub::new();
        match hub.ask("q1") {
            AskOutcome::Pending(rx) => {
                assert!(hub.resolve("q1", "use sqlite".into()));
                assert_eq!(rx.blocking_recv().unwrap(), "use sqlite");
            }
            _ => panic!("expected Pending"),
        }
        assert_eq!(hub.waiting_count(), 0);
    }

    #[test]
    fn resolve_before_ask_is_parked_and_consumed_once() {
        let hub = QuestionHub::new();
        assert!(hub.resolve("q2", "early".into()));
        match hub.ask("q2") {
            AskOutcome::Answered(a) => assert_eq!(a, "early"),
            _ => panic!("expected Answered"),
        }
        // Consumed: a second ask for the same id must not see the stale answer.
        assert!(matches!(hub.ask("q2"), AskOutcome::Pending(_)));
    }

    #[test]
    fn abandon_closes_the_channel_without_early_residue() {
        let hub = QuestionHub::new();
        match hub.ask("q3") {
            AskOutcome::Pending(mut rx) => {
                hub.abandon("q3");
                assert!(rx.try_recv().is_err());
            }
            _ => panic!("expected Pending"),
        }
        // abandon must not park anything: the next ask for this id waits.
        assert!(matches!(hub.ask("q3"), AskOutcome::Pending(_)));
    }

    #[test]
    fn attach_flag_toggles() {
        let hub = QuestionHub::new();
        assert!(!hub.is_attached());
        hub.attach();
        assert!(hub.is_attached());
    }

    /// Pins the description semantics (rules/01): the per-turn cap is gone —
    /// several calls per turn are fine (one question per call) — while the
    /// "genuinely ambiguous" gate survives to prevent over-asking.
    #[test]
    fn description_allows_several_questions_per_turn() {
        let tool = QuestionTool::new(QuestionHub::new());
        let d = tool.description();
        assert!(!d.contains("at most one"), "per-turn cap must be gone: {d}");
        assert!(
            d.contains("several in one turn"),
            "batched wording must be advertised: {d}"
        );
        assert!(
            d.contains("genuinely ambiguous"),
            "gate wording must survive: {d}"
        );
    }

    #[tokio::test]
    async fn execute_without_listener_returns_fallback_immediately() {
        let hub = QuestionHub::new(); // never attached
        let tool = QuestionTool::new(hub);
        let out = tool.execute(input("which db?"), &ctx("q4")).await.unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, NO_LISTENER_REPLY);
    }

    #[tokio::test]
    async fn execute_missing_question_is_an_error() {
        let hub = QuestionHub::new();
        hub.attach();
        let tool = QuestionTool::new(hub);
        let out = tool
            .execute(serde_json::json!({ "options": ["a"] }), &ctx("q5"))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("question"));
    }

    #[tokio::test]
    async fn execute_resolved_answer_becomes_the_tool_result() {
        let hub = QuestionHub::new();
        hub.attach();
        let tool = QuestionTool::new(hub.clone());
        let cx6 = ctx("q6");
        let fut = tool.execute(input("which db?"), &cx6);
        // Resolve as soon as the tool has registered (bounded polling).
        let resolver = tokio::spawn(async move {
            for _ in 0..500 {
                if hub.resolve("q6", "postgres".into()) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("tool never asked");
        });
        let out = fut.await.unwrap();
        resolver.await.unwrap();
        assert_eq!(out.content, "postgres");
        assert!(!out.is_error);
    }

    #[tokio::test]
    async fn cancelled_future_does_not_leak_registration() {
        let hub = QuestionHub::new();
        hub.attach();
        let tool = QuestionTool::new(hub.clone());
        let cx7 = ctx("q7");
        let mut fut = tool.execute(input("q?"), &cx7);
        // Poll once so `ask` registers, then drop like the cancel race does.
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let _ = fut.as_mut().poll(&mut cx);
        assert_eq!(hub.waiting_count(), 1);
        drop(fut);
        assert_eq!(hub.waiting_count(), 0);
    }
}
