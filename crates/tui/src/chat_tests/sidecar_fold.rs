//! Folding tests for the `/sidecar` Q/A block: Start/Child/Turn lifecycle,
//! bare-`LlmUsage` cost accounting to the parent, focus semantics and the
//! header-only flatten contract. Mirrors `chat_tests/subagent.rs` style.

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

/// The sidecar block currently in the view (last one wins).
fn sidecar_block(v: &ChatView) -> Option<&ChatBlock> {
    v.blocks
        .iter()
        .rev()
        .find(|b| matches!(b, ChatBlock::Sidecar { .. }))
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

/// `SidecarStart` pushes exactly one `Sidecar` block and auto-focuses the
/// sidecar box (the user lands in it without an extra keystroke).
#[test]
fn sidecar_start_pushes_block_and_auto_focuses() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "这段代码做什么?"));
    assert!(sidecar_block(&v).is_some(), "block must be pushed");
    assert!(v.sidecar_focus, "Start must auto-focus the sidecar box");
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar {
            question, done, ok, ..
        }) => {
            assert_eq!(question, "这段代码做什么?");
            assert!(!done, "a fresh block is still running");
            assert!(!ok);
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }
}

/// A second conversation (post `/task` switch rebuild) opens its own block —
/// the old one is never re-focused by the new Start.
#[test]
fn second_start_pushes_a_second_block() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "a1", 10, 1));
    v.apply(&sc_start("sc-2", "q2"));
    let count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Sidecar { .. }))
        .count();
    assert_eq!(count, 2);
}

/// Child text streams into the block's nested view only — the parent
/// transcript grows no Assistant block from sidecar content, and the parent's
/// flat transcript carries the header row only (the focused body is swapped
/// in by `compute_display`).
#[test]
fn sidecar_child_text_streams_into_block_not_parent() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&sc_child(
        "sc-1",
        SessionEvent::TextDelta("旁路回答 alpha".into()),
    ));
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar { view, done, .. }) => {
            assert!(
                block_text(view).contains("旁路回答 alpha"),
                "child delta must land in the nested view"
            );
            assert_eq!(view.blocks.len(), 1, "delta creates exactly one block");
            assert!(!done, "no Turn yet");
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }
    assert!(
        !block_text(&v).contains("旁路回答 alpha"),
        "the flat parent transcript shows the header only"
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
    assert!(
        !block_text(&v).contains("ghost"),
        "frames of unknown sidecar ids must be dropped"
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

/// `SidecarTurn` finalizes the block: status, answer summary, and the
/// per-conversation running totals (tokens/rounds accumulate across
/// follow-up turns).
#[test]
fn sidecar_turn_finalizes_and_follow_ups_accumulate() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "第一个答案", 100, 1));
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar {
            done,
            ok,
            answer,
            total_tokens,
            rounds,
            elapsed_ms,
            ..
        }) => {
            assert!(*done);
            assert!(*ok);
            assert_eq!(answer.as_deref(), Some("第一个答案"));
            assert_eq!(*total_tokens, 100);
            assert_eq!(*rounds, 1);
            assert_eq!(*elapsed_ms, 42);
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }

    // Follow-up on the SAME conversation: the block is reused and totals run.
    v.apply(&sc_turn("sc-1", true, "第二个答案", 30, 1));
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar {
            answer,
            total_tokens,
            rounds,
            ..
        }) => {
            assert_eq!(answer.as_deref(), Some("第二个答案"), "latest answer wins");
            assert_eq!(*total_tokens, 130, "per-conversation running sum");
            assert_eq!(*rounds, 2);
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }
    assert!(v.sidecar_focus, "Turn never steals the focus away");
}

/// A failed turn keeps the block visible with its error summary and a
/// non-empty-answer contract (empty answers never overwrite a previous one).
#[test]
fn failed_turn_keeps_previous_answer_and_reports_failure() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q"));
    v.apply(&sc_turn("sc-1", true, "好的", 10, 1));
    v.apply(&sc_turn("sc-1", false, "", 0, 0));
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar {
            done, ok, answer, ..
        }) => {
            assert!(*done);
            assert!(!*ok, "failure must be visible in the header status");
            assert_eq!(
                answer.as_deref(),
                Some("好的"),
                "an empty failure answer must not wipe the previous one"
            );
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }
}

/// The flat main transcript carries ZERO sidecar lines: the focused body is
/// swapped in by `compute_display` and exit purges the block — the bypass
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

/// `purge` (the ESC / Ctrl+L exit path) removes every sidecar block and
/// drops the focus — the destroy contract the transcript relies on.
#[test]
fn purge_removes_every_sidecar_block_and_the_focus() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "q1"));
    v.apply(&sc_turn("sc-1", true, "a1", 10, 1));
    v.apply(&sc_start("sc-2", "q2"));
    assert!(v.sidecar_focus);

    crate::chat::sidecar::purge(&mut v);
    assert!(
        !v.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Sidecar { .. })),
        "every sidecar block is gone"
    );
    assert!(!v.sidecar_focus, "focus is released");
}

/// The panel's Start adopts the placeholder in place: one block total, the
/// placeholder's empty id replaced by the real conversation id.
#[test]
fn start_adopts_the_placeholder_in_place() {
    let (mut v, _tx) = open_panel();
    v.apply(&sc_start("sc-1", "这段代码做什么?"));
    let count = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Sidecar { .. }))
        .count();
    assert_eq!(count, 1, "placeholder adopted, not a second block");
    match sidecar_block(&v) {
        Some(ChatBlock::Sidecar { id, question, .. }) => {
            assert_eq!(id, "sc-1");
            assert_eq!(question, "这段代码做什么?");
        }
        other => panic!("expected Sidecar block, got {other:?}"),
    }
}

/// A Start arriving while the panel is CLOSED (exit / `/task` switch) is a
/// late frame from a destroyed conversation — swallowed, no block, no focus.
#[test]
fn start_with_closed_panel_is_swallowed() {
    let mut v = ChatView {
        sidecar_focus: false,
        ..ChatView::default()
    };
    v.apply(&sc_start("sc-1", "迟到的问题"));
    assert!(
        sidecar_block(&v).is_none(),
        "late Start must not push a zombie block"
    );
    assert!(!v.sidecar_focus, "and must not steal the focus");
}
