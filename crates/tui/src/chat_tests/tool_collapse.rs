//! Step-group folding for runs of tool calls (`ChatBlock::StepGroup`).
//!
//! A group is the canonical ladder for one admitted user Turn; assistant
//! text, image, marker, and subagent presentation do not split it. Within a
//! group a step is one Thinking run plus every function call that follows;
//! sequential and parallel calls both accumulate until new Thinking opens
//! the next step. Default render is ONE
//! clickable group row `▸ N Steps`; clicking it lists the step rows,
//! clicking a step row reveals its thinking + `N Function calls`, opening
//! that aggregate lists calls, and clicking a call row expands only that
//! call's result; Ctrl+L collapses every level again.

use super::super::*;

/// Collect `(group_idx, steps)` for every StepGroup in the view.
fn groups(v: &ChatView) -> Vec<(usize, &Vec<Step>)> {
    v.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::StepGroup { steps, .. } => Some((i, steps)),
            _ => None,
        })
        .collect()
}

/// Flatten one group's steps into its calls in order.
fn group_calls(v: &ChatView) -> Vec<Vec<&ToolCall>> {
    groups(v)
        .iter()
        .map(|(_, steps)| steps.iter().flat_map(|s| s.calls.iter()).collect())
        .collect()
}

fn flatten_text(v: &ChatView) -> Vec<String> {
    v.flatten()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn parallel_tool_calls_form_one_group_and_route_by_id() {
    // Regression: when two tools start before either ends (parallel bash
    // calls), they join ONE group (and one step, neither being finished at
    // the second start) and each ToolEnd must append output to its own call
    // by id — not to the last-pushed call.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo A"}),
    });
    v.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo B"}),
    });
    // End out of call order: B finishes first, then A.
    v.apply(&SessionEvent::ToolEnd {
        id: "b".into(),
        name: "bash".into(),
        output: "B-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A-out".into(),
        is_error: false,
        images: Vec::new(),
    });

    let grps = group_calls(&v);
    assert_eq!(grps.len(), 1, "concurrent calls must form one group");
    assert_eq!(grps[0].len(), 2, "the step must hold both calls");
    // Calls keep start order regardless of end order.
    assert_eq!(grps[0][0].id, "a");
    assert_eq!(grps[0][1].id, "b");
    let text = |c: &ToolCall| -> String {
        c.header
            .spans
            .iter()
            .chain(c.output.iter().flat_map(|l| l.spans.iter()))
            .map(|s| s.content.clone())
            .collect()
    };
    let text_a = text(grps[0][0]);
    let text_b = text(grps[0][1]);
    assert!(text_a.contains("echo A"), "call A header: {text_a}");
    assert!(text_a.contains("A-out"), "call A output: {text_a}");
    assert!(!text_a.contains("B-out"), "call A contaminated: {text_a}");
    assert!(text_b.contains("echo B"), "call B header: {text_b}");
    assert!(text_b.contains("B-out"), "call B output: {text_b}");
    assert!(!text_b.contains("A-out"), "call B contaminated: {text_b}");
    // Finished calls record elapsed time.
    assert!(grps[0].iter().all(|c| c.elapsed_ms.is_some()));
}

#[test]
fn sequential_calls_without_new_thinking_stay_in_one_step() {
    // Call completion is not a Step boundary. Until a new Thinking run
    // appears, sequential calls accumulate in the same function-call list.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo A"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "A-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo B"}),
    });
    let grps = groups(&v);
    assert_eq!(grps.len(), 1, "sequential calls stay in one group");
    assert_eq!(grps[0].1.len(), 1, "calls alone do not create steps");
    assert_eq!(grps[0].1[0].calls[0].id, "a");
    assert_eq!(grps[0].1[0].calls[1].id, "b");
}

#[test]
fn new_thinking_opens_step_even_when_previous_call_is_still_running() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "sleep 1"}),
    });
    v.apply(&SessionEvent::ReasoningDelta("next thought".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "b".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo B"}),
    });

    let steps = groups(&v)[0].1;
    assert_eq!(steps.len(), 2, "new Thinking is the Step boundary");
    assert_eq!(steps[0].calls[0].id, "a");
    assert_eq!(steps[1].calls[0].id, "b");
    assert!(steps[1].thinking_raw.contains("next thought"));
}

#[test]
fn collapsed_by_default_renders_single_group_row() {
    let mut v = ChatView::default();
    for id in ["a", "b"] {
        v.apply(&SessionEvent::ReasoningDelta(format!("think {id}")));
        v.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": format!("echo {id}")}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: format!("{id}-out\nsecond line"),
            is_error: false,
            images: Vec::new(),
        });
    }
    let lines = flatten_text(&v);
    // Group row + trailing blank: the whole ladder is one clickable row.
    assert_eq!(lines.len(), 2, "default render: group row + blank");
    assert!(
        lines[0].contains("\u{25b8} 2 Steps"),
        "collapsed group row carries the step count: {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("\u{276f}"),
        "closed group must use the closed glyph: {:?}",
        lines[0]
    );
    // Step rows, call headers and outputs are all folded away.
    assert!(!lines.iter().any(|l| l.contains("Step(")));
    assert!(!lines.iter().any(|l| l.contains("echo a")));
    assert!(!lines.iter().any(|l| l.contains("a-out")));

    // Singular grammar for a single-step group (two parallel calls = one
    // step).
    let mut v1 = ChatView::default();
    v1.apply(&SessionEvent::ToolStart {
        id: "solo".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "true"}),
    });
    v1.apply(&SessionEvent::ToolEnd {
        id: "solo".into(),
        name: "bash".into(),
        output: "ok".into(),
        is_error: false,
        images: Vec::new(),
    });
    let solo = flatten_text(&v1);
    assert_eq!(solo.len(), 2, "group row + blank");
    assert!(
        solo[0].contains("1 Step") && !solo[0].contains("Steps"),
        "single step uses singular: {:?}",
        solo[0]
    );
}

#[test]
fn running_hint_persists_until_say_begins() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "slow".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "sleep 5"}),
    });
    let line = &flatten_text(&v)[0];
    assert!(
        line.contains("running"),
        "unfinished call must show the running hint: {line:?}"
    );
    assert!(
        line.contains("Step  \u{280b} running"),
        "the animation must keep a two-column gap: {line:?}"
    );
    v.apply(&SessionEvent::ToolEnd {
        id: "slow".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });
    let line = &flatten_text(&v)[0];
    assert!(
        line.contains("running"),
        "ToolEnd must keep progress alive while waiting for Say: {line:?}"
    );
    v.apply(&SessionEvent::TextDelta("final answer".into()));
    let line = &flatten_text(&v)[0];
    assert!(
        !line.contains("running"),
        "the first non-empty Say chunk must settle progress: {line:?}"
    );

    // Say is terminal for the ladder: later frames cannot re-arm it.
    v.apply(&SessionEvent::ReasoningDelta("one more check".into()));
    assert!(!flatten_text(&v)[0].contains("running"));
    v.apply(&SessionEvent::ToolStart {
        id: "after-say".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "true"}),
    });
    assert!(!flatten_text(&v)[0].contains("running"));
}

#[test]
fn terminal_event_clears_progress_when_no_say_arrives() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "no-say".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "true"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "no-say".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });
    assert!(flatten_text(&v)[0].contains("running"));
    v.apply(&SessionEvent::Done);
    assert!(!flatten_text(&v)[0].contains("running"));

    let mut failed = ChatView::default();
    failed.apply(&SessionEvent::ReasoningDelta("will fail".into()));
    assert!(flatten_text(&failed)[0].contains("running"));
    failed.apply(&SessionEvent::Error("boom".into()));
    assert!(!flatten_text(&failed)[0].contains("running"));

    let mut recovered = ChatView::default();
    recovered.apply(&SessionEvent::ToolStart {
        id: "recovered".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "true"}),
    });
    recovered.apply(&SessionEvent::ToolEnd {
        id: "recovered".into(),
        name: "bash".into(),
        output: "done".into(),
        is_error: false,
        images: Vec::new(),
    });
    recovered.reconcile_completed_assistant("reliable answer");
    assert!(!flatten_text(&recovered)[0].contains("running"));
}

#[test]
fn toggling_the_ladder_reveals_each_level() {
    // Two Thinking + call pairs. Default = group row + blank = 2;
    // group open = + 2 step rows = 4; step 1 open = Thinking header/body +
    // calls row = 7; calls open = + call row = 8; call expanded = + output = 9
    // plus the trailing blank = 10 total.
    let mut v = ChatView::default();
    for id in ["a", "b"] {
        v.apply(&SessionEvent::ReasoningDelta(format!("think {id}")));
        v.apply(&SessionEvent::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": format!("echo {id}")}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: id.into(),
            name: "bash".into(),
            output: format!("{id}-out"),
            is_error: false,
            images: Vec::new(),
        });
    }
    let list = flatten_text(&v);
    assert_eq!(list.len(), 2, "default = group row + blank");
    assert!(
        list[0].contains("\u{25b8} 2 Steps"),
        "collapsed group row: {:?}",
        list[0]
    );

    // Open the group: both closed step rows appear.
    v.toggle_tool_call_at(0, 0);
    let open_group = flatten_text(&v);
    assert_eq!(open_group.len(), 4);
    assert!(
        open_group[0].contains("\u{276f} 2 Steps"),
        "open group glyph: {:?}",
        open_group[0]
    );
    assert!(
        open_group[1].contains("\u{25b8} Step(1)") && open_group[2].contains("\u{25b8} Step(2)"),
        "both steps render collapsed rows: {open_group:?}"
    );

    // Open step 1: thinking/calls summary appears, but call rows stay hidden.
    v.toggle_tool_call_at(0, 1);
    let open_step = flatten_text(&v);
    assert_eq!(open_step.len(), 7);
    assert!(
        open_step[1].contains("\u{276f} Step(1)"),
        "opened step row: {:?}",
        open_step[1]
    );
    assert!(
        open_step[4].contains("1 Function call"),
        "function-call aggregate row: {:?}",
        open_step[4]
    );
    assert!(!open_step.join("\n").contains("echo a"));

    // Open the aggregate: the call row appears without its result.
    v.toggle_tool_call_at(0, 2);
    let calls_open = flatten_text(&v);
    assert_eq!(calls_open.len(), 8);
    assert!(calls_open[5].contains("echo a"));
    assert!(!calls_open.join("\n").contains("a-out"));

    // Click the function call: its result appears.
    v.toggle_tool_call_at(0, 3);
    let expanded = flatten_text(&v);
    assert_eq!(expanded.len(), 10, "call result + blank join the ladder");
    assert!(
        expanded[6].contains("a-out"),
        "output visible: {:?}",
        expanded[6]
    );

    // Re-toggling the group collapses the whole ladder; the group row stays.
    v.toggle_tool_call_at(0, 0);
    let closed = flatten_text(&v);
    assert_eq!(closed.len(), 2, "group closed folds every level");
    assert!(
        closed[0].contains("\u{25b8} 2 Steps"),
        "group row persists: {closed:?}"
    );
}

#[path = "tool_collapse/turn_routing.rs"]
mod turn_routing;
