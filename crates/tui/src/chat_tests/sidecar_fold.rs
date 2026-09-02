//! Folding tests for the `/sidecar` Q/A panel: Start/Child/Turn lifecycle,
//! bare-`LlmUsage` cost accounting to the parent, focus semantics and the
//! zero-lines flatten contract. Mirrors `chat_tests/subagent.rs` style.

use super::*;

/// Render a styled `Line` to plain text (span contents concatenated).
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn sc_start(id: &str, question: &str) -> SessionEvent {
    SessionEvent::SidecarStart {
        id: id.into(),
        question: question.into(),
    }
}

fn sc_child(id: &str, ev: SessionEvent) -> SessionEvent {
    SessionEvent::SidecarChild {
        id: id.into(),
        ev: Box::new(ev),
    }
}

fn sc_turn(id: &str, ok: bool, answer: &str, tokens: u64, rounds: usize) -> SessionEvent {
    SessionEvent::SidecarTurn {
        id: id.into(),
        ok,
        answer: answer.into(),
        elapsed_ms: 42,
        total_tokens: tokens,
        rounds,
    }
}

/// The sidecar panel currently open on the view (field `ChatView::sidecar`).
fn panel(v: &ChatView) -> Option<&SidecarPanel> {
    v.sidecar.as_ref()
}

/// Mirror the real entry: only an OPEN panel (placeholder + focus) accepts
/// `SidecarStart` frames — every conversation test opens the panel first.
fn open_panel() -> (
    ChatView,
    tokio::sync::mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
) {
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::sidecar_ui::SidecarCmd>(1);
    let mut v = ChatView::default();
    crate::sidecar_ui::enter_panel(&mut v, &tx);
    (v, tx)
}

/// `SidecarStart` claims exactly one panel (field `sidecar`) and
/// auto-focuses the sidecar box (the user lands in it without an extra
/// keystroke).
#[test]
fn sidecar_start_claims_the_panel_and_auto_focuses() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "这段代码做什么?"));
    assert!(
        panel(&v).is_some(),
        "panel must be claimed on the view field"
    );
    assert!(v.sidecar_focus, "Start must auto-focus the sidecar box");
    let p = panel(&v).expect("panel present");
    assert_eq!(p.id, "sc-1");
    assert_eq!(p.question, "这段代码做什么?");
    assert!(!p.done, "a fresh panel is still running");
    assert!(!p.ok);
}

/// A second conversation (post `/task` switch rebuild) takes over the single
/// panel — and the old conversation's late frames are swallowed.
#[test]
fn second_start_replaces_the_panel() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "a1", 10, 1));
    v.apply(&sc_start("sc-2", "q2"));
    assert!(v.sidecar_focus, "the fresh panel is focused");
    let p = panel(&v).expect("panel holds the new conversation");
    assert_eq!(p.id, "sc-2");
    assert_eq!(p.question, "q2");
    // A late Turn for the replaced conversation must not touch the panel.
    v.apply(&sc_turn("sc-1", true, "late", 99, 9));
    let p = panel(&v).expect("panel still present");
    assert_eq!(
        p.answer.as_deref(),
        None,
        "late Turn for a stale id is swallowed"
    );
    assert_eq!(p.total_tokens, 0, "no late tokens land on the new panel");
    assert_eq!(p.rounds, 0);
}

/// Child text streams into the panel's nested view only — the parent
/// transcript grows no Assistant block from sidecar content, and the parent's
/// flat transcript carries nothing from the panel (the focused body is
/// swapped in by `compute_display`).
#[test]
fn sidecar_child_text_streams_into_block_not_parent() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&sc_child(
        "sc-1",
        SessionEvent::TextDelta("旁路回答 alpha".into()),
    ));
    let p = panel(&v).expect("panel present");
    assert!(
        block_text(&p.view).contains("旁路回答 alpha"),
        "child delta must land in the panel's nested view"
    );
    assert_eq!(p.view.blocks.len(), 1, "delta creates exactly one block");
    assert!(!p.done, "no Turn yet");
    assert!(
        !block_text(&v).contains("旁路回答 alpha"),
        "the flat parent transcript shows nothing from the panel"
    );
}

/// Child deltas target their own conversation: an id mismatch is swallowed
/// (a late frame after a `/task` switch must not corrupt the new view).
#[test]
fn sidecar_child_for_unknown_id_is_swallowed() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&sc_child(
        "sc-other",
        SessionEvent::TextDelta("ghost".into()),
    ));
    let p = panel(&v).expect("panel present");
    assert!(
        !block_text(&p.view).contains("ghost"),
        "frames of unknown sidecar ids must be dropped"
    );
    assert!(
        !block_text(&v).contains("ghost"),
        "and nothing leaks into the parent transcript"
    );
}

/// The child's `LlmUsage` arrives **wrapped** in `SidecarChild` — the parent
/// arm must skip it so the token is not counted twice (the bare forward
/// already accumulated it).
#[test]
fn sidecar_child_llm_usage_is_not_double_counted() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    let parent_before = v.tokens_total;
    let ctx_before = v.real_context_tokens;
    v.apply(&sc_child(
        "sc-1",
        SessionEvent::LlmUsage {
            total_tokens: 500,
            input_tokens: 400,
            output_tokens: 100,
        },
    ));
    assert_eq!(
        v.tokens_total, parent_before,
        "wrapped child usage must not inflate the parent total"
    );
    assert_eq!(
        v.real_context_tokens, ctx_before,
        "a child round is not part of the parent's context window"
    );
}

/// The bare (unwrapped) `LlmUsage` the actor forwards IS a parent event:
/// it accumulates `tokens_total` — that is the cost-accounting contract.
#[test]
fn bare_llm_usage_accounts_sidecar_cost_to_the_parent() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 500,
        input_tokens: 400,
        output_tokens: 100,
    });
    assert_eq!(v.tokens_total, 500, "sidecar cost lands on the main task");
}

/// `SidecarTurn` finalizes the panel: status, answer summary, and the
/// per-conversation running totals (tokens/rounds accumulate across
/// follow-up turns).
#[test]
fn sidecar_turn_finalizes_and_follow_ups_accumulate() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "第一个答案", 100, 1));
    let p = panel(&v).expect("panel present");
    assert!(p.done);
    assert!(p.ok);
    assert_eq!(p.answer.as_deref(), Some("第一个答案"));
    assert_eq!(p.total_tokens, 100);
    assert_eq!(p.rounds, 1);
    assert_eq!(p.elapsed_ms, 42);

    // Follow-up on the SAME conversation: the panel is reused and totals run.
    v.apply(&sc_turn("sc-1", true, "第二个答案", 30, 1));
    let p = panel(&v).expect("panel present");
    assert_eq!(
        p.answer.as_deref(),
        Some("第二个答案"),
        "latest answer wins"
    );
    assert_eq!(p.total_tokens, 130, "per-conversation running sum");
    assert_eq!(p.rounds, 2);
    assert!(v.sidecar_focus, "Turn never steals the focus away");
}

/// A failed turn keeps the panel visible with its error summary and a
/// non-empty-answer contract (empty answers never overwrite a previous one).
#[test]
fn failed_turn_keeps_previous_answer_and_reports_failure() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&sc_turn("sc-1", true, "好的", 10, 1));
    v.apply(&sc_turn("sc-1", false, "", 0, 0));
    let p = panel(&v).expect("panel present");
    assert!(p.done);
    assert!(!p.ok, "failure must be visible in the header status");
    assert_eq!(
        p.answer.as_deref(),
        Some("好的"),
        "an empty failure answer must not wipe the previous one"
    );
}

/// The flat main transcript carries ZERO sidecar lines: the focused body is
/// swapped in by `compute_display` and exit clears the panel — the bypass
/// Q/A is not a transcript artifact.
#[test]
fn flatten_emits_zero_lines_for_sidecar_blocks() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "进度怎么样?"));
    v.apply(&sc_child(
        "sc-1",
        SessionEvent::TextDelta("内部delta".into()),
    ));
    let running = v.flatten_with(0, 1_000);
    assert!(
        !running.iter().any(|l| line_text(l).contains("sidecar")),
        "running panel leaves no flat trace"
    );

    v.apply(&sc_turn("sc-1", true, "全部完成", 70, 1));
    let done_lines = v.flatten_with(0, 1_000);
    assert!(
        !done_lines.iter().any(|l| line_text(l).contains("sidecar")),
        "finished panel leaves no flat trace either"
    );
}

/// `purge` (the ESC / Ctrl+L exit path) clears the panel field and drops the
/// focus — the destroy contract the transcript relies on.
#[test]
fn purge_clears_the_panel_and_the_focus() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "a1", 10, 1));
    v.apply(&sc_start("sc-2", "q2"));
    assert!(v.sidecar_focus);

    crate::chat::sidecar::purge(&mut v);
    assert!(v.sidecar.is_none(), "the panel field is cleared");
    assert!(!v.sidecar_focus, "focus is released");
}

/// The panel's Start adopts the fresh placeholder in place: exactly one
/// panel on the view, its empty id replaced by the real conversation id.
#[test]
fn start_adopts_the_placeholder_in_place() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "这段代码做什么?"));
    let p = v.sidecar.as_ref().expect("exactly one panel on the view");
    assert_eq!(p.id, "sc-1");
    assert_eq!(p.question, "这段代码做什么?");
}

/// A Start arriving while the panel is CLOSED (exit / `/task` switch) is a
/// late frame from a destroyed conversation — swallowed, no panel, no focus.
#[test]
fn start_with_closed_panel_is_swallowed() {
    let mut v = ChatView {
        sidecar_focus: false,
        ..ChatView::default()
    };
    v.apply(&sc_start("sc-1", "迟到的问题"));
    assert!(
        v.sidecar.is_none(),
        "late Start must not claim a zombie panel"
    );
    assert!(!v.sidecar_focus, "and must not steal the focus");
}

// ----- Instant question echo (`sidecar_ui::echo_question`) -----

/// Nested view's rendered text, markers included.
fn nested_text(v: &ChatView) -> String {
    match v.sidecar.as_ref() {
        Some(p) => p
            .view
            .blocks
            .iter()
            .flat_map(|b| match b {
                ChatBlock::User { rendered } => {
                    rendered.iter().map(|l| line_text(l)).collect::<Vec<_>>()
                }
                ChatBlock::Marker(lines) => lines.iter().map(|l| line_text(l)).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => panic!("expected an open sidecar panel, got {other:?}"),
    }
}

/// The echo lands in the placeholder's nested view immediately: a
/// markdown-rendered `ChatBlock::User` followed by a blank marker — the same
/// shape the main transcript's `push_user` produces.
#[test]
fn echo_question_lands_in_placeholder_view() {
    let (mut v, _tx) = open_panel();
    crate::sidecar_ui::echo_question(&mut v, "**加粗**的问题");
    let text = nested_text(&v);
    assert!(
        text.contains("加粗") && text.contains("的问题"),
        "question text must render into the nested view, got {text:?}"
    );
    // Markdown echo: the literal asterisks are consumed by the renderer.
    assert!(
        !text.contains("**"),
        "echo goes through markdown::render, got {text:?}"
    );
}

/// `SidecarStart` adopts the placeholder IN PLACE (nested view kept), so the
/// pre-Start echo survives exactly once — the lifecycle frames never re-echo
/// the question.
#[test]
fn echo_survives_start_adoption_without_duplication() {
    let (mut v, _tx) = open_panel();
    crate::sidecar_ui::echo_question(&mut v, "第一问");
    v.apply(&sc_start("sc-1", "第一问"));
    assert_eq!(
        nested_text(&v).matches("第一问").count(),
        1,
        "echo exactly once after adoption"
    );
}

/// A follow-up echoes into the SAME (already adopted) conversation block —
/// the question reads above the still-streaming answer.
#[test]
fn followup_echo_joins_the_adopted_block() {
    let (mut v, _tx) = open_panel();
    crate::sidecar_ui::echo_question(&mut v, "第一问");
    v.apply(&sc_start("sc-1", "第一问"));
    crate::sidecar_ui::echo_question(&mut v, "追问");
    assert_eq!(
        nested_text(&v).matches("第一问").count() + nested_text(&v).matches("追问").count(),
        2,
        "both echoes visible in the one adopted panel"
    );
}

/// No open panel (exit purged it): a late echo is a no-op, not a panic and
/// not a stray transcript block.
#[test]
fn echo_without_sidecar_block_is_a_noop() {
    let mut v = ChatView::default();
    crate::sidecar_ui::echo_question(&mut v, "孤儿问题");
    assert!(
        v.blocks.is_empty(),
        "nothing may be pushed, got {:?}",
        v.blocks.len()
    );
    assert!(v.sidecar.is_none());
}
