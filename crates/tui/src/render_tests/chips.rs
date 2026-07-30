use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;

/// Issue #6: the `[agent]` status chip is Yellow in plan mode and Cyan
/// for every other agent. Guards against a regression to the old uniform
/// Magenta.
#[test]
fn agent_chip_color_is_yellow_for_plan_cyan_otherwise() {
    assert_eq!(agent_chip_fg("plan"), Color::Yellow);
    assert_eq!(agent_chip_fg("act"), Color::Cyan);
    assert_eq!(agent_chip_fg("explore"), Color::Cyan);
    assert_eq!(agent_chip_fg(""), Color::Cyan);
}

/// Issue #6: the plan/act mode-flash chip background is Yellow for plan,
/// Cyan for act. Both the agent chip and the flash share the same theme
/// mapping, so they never visually disagree.
#[test]
fn mode_flash_bg_matches_plan_yellow_act_cyan() {
    assert_eq!(mode_flash_bg(true), Color::Yellow);
    assert_eq!(mode_flash_bg(false), Color::Cyan);
    // The two theme helpers agree on plan/act, so the chip and flash
    // always render the same hue.
    assert_eq!(agent_chip_fg("plan"), mode_flash_bg(true));
    assert_eq!(agent_chip_fg("act"), mode_flash_bg(false));
}

/// Issue #5 core invariant: while a preamble block is WITHHELD (multiple
/// subagents running), the `header_line_idx` values reported by
/// `thinking_headers()` and `subagent_headers()` must exactly match the
/// line indices in `flatten_with()` where those headers actually render.
/// If any of the `is_withheld` guards in those three functions drift out
/// of sync, a header index would point at the wrong row and mouse clicks
/// would land on the wrong block.
#[test]
fn header_line_indices_aligned_with_flatten_while_withheld() {
    let mut v = ChatView::default();
    // Preamble assistant text — withheld once 2 subagents run. Its "say:"
    // header + 2 content lines mean a stale (non-skipping) accounting
    // would shift every later header by 3 rows.
    v.apply(&SessionEvent::TextDelta(
        "preamble line one\npreamble line two".into(),
    ));
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "pa".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "pb".into(),
        child_session_id: "cb".into(),
    });
    // Thinking block after the subagents: its header_line_idx is the
    // canary — if the withheld preamble were counted it would overshoot.
    v.apply(&SessionEvent::ReasoningDelta(
        "post\ndispatch\nanalysis".into(),
    ));

    assert!(
        v.hidden_assistant_idx.is_some(),
        "preamble must be withheld"
    );
    assert_eq!(v.subagents_running, 2);
    let flat = v.flatten_with(0);

    let line_text =
        |idx: usize| -> String { flat[idx].spans.iter().map(|s| s.content.clone()).collect() };
    // Every thinking header points at a flatten line containing "Thinking".
    let th = v.thinking_headers();
    assert!(!th.is_empty());
    for h in &th {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("Thinking"),
            "thinking header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // Every subagent header points at a flatten line containing "subagent".
    let sh = v.subagent_headers();
    assert_eq!(sh.len(), 2);
    for h in &sh {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("subagent"),
            "subagent header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // No two headers collide on the same rendered line.
    let mut all_idx: Vec<usize> = th.iter().map(|h| h.header_line_idx).collect();
    all_idx.extend(sh.iter().map(|h| h.header_line_idx));
    let mut sorted = all_idx.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all_idx.len(), "collide: {:?}", all_idx);
    // The withheld preamble contributes ZERO lines to flatten.
    for (i, line) in flat.iter().enumerate() {
        let txt: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            !txt.contains("preamble line"),
            "line {i}: withheld preamble leaked: {:?}",
            txt,
        );
    }
}
