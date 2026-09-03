use super::*;

#[test]
fn text_between_calls_stays_in_one_turn_and_routes_by_id() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "one"}),
    });
    v.apply(&SessionEvent::ToolStart {
        id: "parallel".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "parallel"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "parallel".into(),
        name: "bash".into(),
        output: "parallel-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    v.apply(&SessionEvent::TextDelta("thinking out loud".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "t2".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "two"}),
    });
    // Say is presentation inside the same admitted turn. It is not Thinking,
    // so t2 remains in the same Step and the same function-call aggregate.
    let grps = group_calls(&v);
    assert_eq!(grps.len(), 1, "one admitted turn owns one group");
    assert_eq!(grps[0].len(), 3);
    assert_eq!(groups(&v)[0].1.len(), 1, "only new Thinking opens Step(2)");

    // Ending the older group's call after the newer group exists must still
    // route into the older group by id.
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "first-out".into(),
        is_error: false,
        images: Vec::new(),
    });
    assert!(
        group_calls(&v)[0][0]
            .output
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("first-out"))),
        "output must land in the older group's call"
    );
    assert_eq!(groups(&v).len(), 1);
}

#[test]
fn orphan_tool_end_creates_synthetic_group() {
    // A ToolEnd with no preceding ToolStart (e.g. a lost event) must not
    // panic; it creates a synthetic finished single-call group carrying the
    // id and "(output)" header.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolEnd {
        id: "orphan".into(),
        name: "bash".into(),
        output: "loose output".into(),
        is_error: false,
        images: Vec::new(),
    });
    let grps = group_calls(&v);
    assert_eq!(grps.len(), 1, "orphan ToolEnd creates one group");
    assert_eq!(grps[0].len(), 1);
    let call = grps[0][0];
    assert_eq!(call.id, "orphan");
    let header: String = call
        .header
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(header.contains("(output)"), "synthetic header: {header}");
    let out: String = call
        .output
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect();
    assert!(out.contains("loose output"), "output appended: {out}");
    assert!(
        call.elapsed_ms.is_some(),
        "synthetic call is finished (no running hint)"
    );
}

#[test]
fn tool_end_error_colors_output_red() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "e1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "false"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "e1".into(),
        name: "bash".into(),
        output: "boom".into(),
        is_error: true,
        images: Vec::new(),
    });
    let call = group_calls(&v)[0][0];
    let span = &call
        .output
        .first()
        .expect("output line")
        .spans
        .first()
        .expect("output span");
    assert!(
        span.style.fg == Some(ratatui::style::Color::Red),
        "error output must be err-colored, got {:?}",
        span.style
    );
}

#[test]
fn collapse_all_collapsible_resets_groups_and_thinking() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t".into(),
        name: "bash".into(),
        output: "out".into(),
        is_error: false,
        images: Vec::new(),
    });
    // A pure-text round keeps a standalone Thinking block (a tool round
    // would absorb its thinking into the step).
    v.apply(&SessionEvent::ReasoningDelta("reason".into()));
    v.apply(&SessionEvent::TextDelta("spoken".into()));
    v.apply(&SessionEvent::Done);
    // Expand everything so they are observably NOT collapsed beforehand.
    for h in v.thinking_headers() {
        v.toggle_thinking_at(h.block_idx);
    }
    for b in v.blocks.iter_mut() {
        if let ChatBlock::StepGroup { open, steps, .. } = b {
            *open = true;
            steps[0].open = true;
            steps[0].calls_open = true;
            steps[0].calls[0].expanded = true; // call output expanded
        }
    }
    v.collapse_all_collapsible();
    for b in &v.blocks {
        match b {
            ChatBlock::Thinking { collapsed, .. } => {
                assert!(*collapsed, "thinking must be collapsed");
            }
            ChatBlock::StepGroup { open, steps, .. } => {
                assert!(
                    !*open
                        && steps.iter().all(|s| {
                            !s.open && !s.calls_open && s.calls.iter().all(|c| !c.expanded)
                        }),
                    "every ladder level must be collapsed after Ctrl+L"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn step_row_line_index_lands_on_the_step_row() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("preamble\nsecond".into()));
    v.apply(&SessionEvent::Done);
    v.apply(&SessionEvent::ToolStart {
        id: "t".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    // Collapsed ladder: exactly one target — the group row. The existing Say
    // makes the Step terminal, so a later call cannot re-arm its spinner.
    let (group_idx, group_line) = {
        let headers = v.tool_call_headers();
        assert_eq!(headers.len(), 1, "expected exactly the group-row header");
        let line: String = v.flatten()[headers[0].header_line_idx]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        (headers[0].header_line_idx, line)
    };
    assert!(
        group_line.contains("1 Step") && !group_line.contains("running"),
        "header_line_idx must land on the group row; got: {group_line:?}"
    );

    // Open the group: the step row becomes the second target and must land
    // exactly one line below the group row.
    let block_idx = groups(&v)[0].0;
    v.toggle_tool_call_at(block_idx, 0);
    let step_idx = v.tool_call_headers()[1].header_line_idx;
    let step_line: String = v.flatten()[step_idx]
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        step_line.contains("Step(1)"),
        "header_line_idx must land on the step row; got: {step_line:?}"
    );
    assert_eq!(
        step_idx,
        group_idx + 1,
        "step row directly follows the group row"
    );
}

#[test]
fn tool_header_keeps_full_command_text() {
    // The header row of a tool call can exceed the terminal width, hiding
    // the real command behind an ellipsis. summarize() must return the full
    // command text so the body layer can wrap it to the terminal width.
    let long_cmd = format!("echo {}", "a".repeat(100));
    assert!(
        long_cmd.chars().count() > 80,
        "test setup: command must exceed 80 cols"
    );
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": long_cmd.clone()}),
    });
    let header = v
        .blocks
        .iter()
        .find_map(|b| match b {
            ChatBlock::StepGroup { steps, .. } => Some(steps[0].calls[0].header.clone()),
            _ => None,
        })
        .expect("step group");
    // spans[0] is the "▸ bash " label; spans[1] is the summarize() output.
    let summary = header.spans[1].content.to_string();
    assert!(
        summary.contains(&long_cmd),
        "header must contain the full command; got {summary:?}"
    );
    assert!(
        !summary.contains('\u{2026}'),
        "header must not be truncated with ellipsis; got {summary:?}"
    );
}

#[test]
fn tool_output_truncated_at_limit() {
    // Even with a fully open ladder, a single ToolEnd event must not capture
    // an unbounded number of lines. The cap (TOOL_OUTPUT_LINES = 200) bounds
    // memory and per-refresh flatten_with cost.
    use crate::chat::TOOL_OUTPUT_LINES;
    let big: String = (0..500).map(|i| format!("line {i}\n")).collect();
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "big".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "yes"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "big".into(),
        name: "bash".into(),
        output: big,
        is_error: false,
        images: Vec::new(),
    });
    let call = group_calls(&v)[0][0];
    assert_eq!(
        call.output.len(),
        TOOL_OUTPUT_LINES,
        "captured output must be capped at TOOL_OUTPUT_LINES"
    );
}
