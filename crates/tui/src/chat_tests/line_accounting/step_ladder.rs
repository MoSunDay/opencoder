use super::*;

#[test]
fn step_group_ladder_depth_alignment() {
    // The StepGroup line accounting must match flatten_with at EVERY ladder
    // depth — this is the invariant that keeps click hit-rects aligned.
    // 2 steps (1 call each, outputs 1 and 2 lines):
    //   group closed                       = 1 + 1
    //   group open, steps closed           = 1 + 2 + 1
    //   group open, steps open            = 1 + (1+1)*2 + 1
    //   calls lists open                  = 1 + (1+1+1)*2 + 1
    //   call "a" expanded                 = previous + output + separator
    let mk = |group_open: bool, open_steps: bool, calls_open: bool, expand: bool| {
        let mut a = tool_call("a", 1);
        a.expanded = expand;
        view_with(vec![
            ChatBlock::StepGroup {
                steps: vec![
                    Step {
                        thinking_raw: String::new(),
                        thinking: Vec::new(),
                        thinking_dirty: false,
                        calls: vec![a],
                        open: open_steps,
                        calls_open,
                        sealed: true,
                    },
                    Step {
                        thinking_raw: String::new(),
                        thinking: Vec::new(),
                        thinking_dirty: false,
                        calls: vec![tool_call("b", 2)],
                        open: open_steps,
                        calls_open,
                        sealed: true,
                    },
                ],
                open: group_open,
                progress_active: false,
            },
            marker_n(1),
        ])
    };

    // Collapsed group (the default): group row + trailing blank only.
    let v = mk(false, false, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 2 + 1);

    // Group open, steps closed: group row + 2 step rows + blank.
    let v = mk(true, false, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 4 + 1);

    // Steps open: thinking plus one calls aggregation row per step.
    let v = mk(true, true, false, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 6 + 1);

    // Calls lists open: one call header per step is now visible.
    let v = mk(true, true, true, false);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 8 + 1);

    // Fully open; only call "a" is expanded so it also renders its 1 output
    // line + separator blank:
    // group(1) + S1(1)+calls(1)+a hdr(1)+a out(1)+sep(1)
    //          + S2(1)+calls(1)+b hdr(1) + trailing blank(1)+marker(1) = 11.
    let v = mk(true, true, true, true);
    assert_line_accounting_matches(&v);
    assert_eq!(v.flatten().len(), 11);
}

#[test]
fn step_group_with_expanded_call_keeps_alignment() {
    // Per-call expansion in an open step: only call "a" shows its
    // output. The recorded hit rows must point at the rendered group/step/
    // call rows, and expanding a call must shift the rows after
    // it by exactly its output + separator.
    let mut a = tool_call("a", 1);
    a.expanded = true;
    let v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![
                Step {
                    thinking_raw: String::new(),
                    thinking: Vec::new(),
                    thinking_dirty: false,
                    calls: vec![a],
                    open: true,
                    calls_open: true,
                    sealed: true,
                },
                Step {
                    thinking_raw: String::new(),
                    thinking: Vec::new(),
                    thinking_dirty: false,
                    calls: vec![tool_call("b", 2)],
                    open: true,
                    calls_open: true,
                    sealed: true,
                },
            ],
            open: true,
            progress_active: false,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    // group(1) + S1(1) + Calls1(1) + a hdr(1) + a out(1) + sep(1)
    // + S2(1) + Calls2(1) + b hdr(1) + trailing blank(1) + marker(1) = 11.
    assert_eq!(v.flatten().len(), 11);

    let call_headers = v.tool_call_headers();
    assert_eq!(
        call_headers.len(),
        7,
        "turn row + 2 step rows + 2 call rows"
    );
    assert_eq!(call_headers[0].call_idx, 0, "group row");
    assert_eq!(call_headers[0].header_line_idx, 0);
    assert_eq!(call_headers[1].call_idx, 1, "step 1 row");
    assert_eq!(call_headers[1].header_line_idx, 1);
    assert_eq!(call_headers[2].call_idx, 2, "calls 1 row");
    assert_eq!(call_headers[2].header_line_idx, 2);
    assert_eq!(call_headers[3].call_idx, 3, "call a's row");
    assert_eq!(call_headers[3].header_line_idx, 3);
    assert_eq!(
        call_headers[4].header_line_idx, 6,
        "step 2 sits after the expanded output + separator"
    );
    assert_eq!(call_headers[5].call_idx, 5, "calls 2 row");
    assert_eq!(call_headers[5].header_line_idx, 7);
    assert_eq!(call_headers[6].call_idx, 6, "call b's header row");
    assert_eq!(call_headers[6].header_line_idx, 8);
    // Each recorded row must be the group row (`{❯|▸} N step(s)`), a step
    // row (`{❯|▸} Step(N)`) or a function-call row — all share the
    // `{❯|▸} ` gutter prefix.
    let flat = v.flatten();
    for h in &call_headers {
        let text: String = flat[h.header_line_idx]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        let t = text.trim_start();
        assert!(
            t.starts_with("\u{25b8} ") || t.starts_with("\u{276f} "),
            "row {} is not a ladder row: {text:?}",
            h.header_line_idx
        );
    }
}

#[test]
fn step_group_running_call_keeps_alignment() {
    // An unfinished call (elapsed_ms == None) streams ONLY in the TURN LIVE
    // REGION while the group is armed (`progress_active`): the transcript
    // gets ZERO ladder rows — no group row, no blank — so the accounting
    // invariant holds with nothing rendered and no hit target exists.
    let v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![Step {
                thinking_raw: String::new(),
                thinking: Vec::new(),
                thinking_dirty: false,
                calls: vec![ToolCall {
                    id: "r".into(),
                    header: Line::from(Span::raw("\u{25b8} bash")),
                    output: Vec::new(),
                    started_at_ms: Some(0),
                    elapsed_ms: None,
                    expanded: false,
                }],
                open: false,
                calls_open: false,
                sealed: false,
            }],
            open: false,
            progress_active: true,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert_eq!(
        flat,
        vec![String::new()],
        "only the marker renders while the turn runs: {flat:?}"
    );
    let live = super::super::step_group::live_lines(&v);
    assert!(
        live.iter().any(|l| l.contains("bash")),
        "the running call streams in the live region instead: {live:?}"
    );
    assert!(
        v.tool_call_headers().is_empty(),
        "no transcript rows -> no targets"
    );
}

/// RUNNING-OPEN LADDER alignment: while the group is armed
/// (`progress_active`) EVEN a user-opened ladder renders ZERO transcript
/// rows — the running turn streams only in the TURN LIVE REGION (current
/// step alone), so no step row leaks and no target exists. On settle the
/// SAME open ladder renders every row and `collect_headers` must count
/// them exactly so hit-rects stay aligned.
#[test]
fn running_open_ladder_keeps_alignment() {
    let mut v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![
                Step {
                    thinking_raw: String::new(),
                    thinking: Vec::new(),
                    thinking_dirty: false,
                    calls: vec![tool_call("a", 1)],
                    open: false,
                    calls_open: false,
                    sealed: true,
                },
                Step {
                    thinking_raw: String::new(),
                    thinking: vec![Line::from("working")],
                    thinking_dirty: false,
                    calls: vec![tool_call("b", 2)],
                    open: true,
                    calls_open: true,
                    sealed: false,
                },
            ],
            open: true,
            progress_active: true,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert_eq!(
        flat,
        vec![String::new()],
        "armed group renders nothing — not even an open ladder's rows: {flat:?}"
    );
    assert!(v.tool_call_headers().is_empty(), "no rows -> no targets");
    let live = super::super::step_group::live_lines(&v);
    assert!(
        live.iter().any(|l| l.contains("Step(2)"))
            && live.iter().any(|l| l.contains("working"))
            && live.iter().any(|l| l.contains("b")),
        "only the CURRENT step streams, in the live region: {live:?}"
    );
    assert!(
        !live.iter().any(|l| l.contains("Step(1)")),
        "the finished step waits inside the group until settle: {live:?}"
    );

    // Settle (`progress_active` flips false): the same open ladder renders
    // every row and the accounting must match again.
    if let ChatBlock::StepGroup {
        progress_active, ..
    } = &mut v.blocks[0]
    {
        *progress_active = false;
    }
    assert_line_accounting_matches(&v);
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert!(
        flat.iter().any(|l| l.contains("Step(1)")),
        "finished step row renders: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("Step(2)")),
        "running step row renders: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("working")),
        "running thinking streams in place: {flat:?}"
    );
    // Ladder block only (the live region is NOT in flatten): group(1) + 1
    // + (1+2+1+1) + trailing blank(1) = 8, marker(1) → 9.
    assert_eq!(v.flatten().len(), 9);
    // Every rendered row is a clickable target in visual order: group,
    // Step(1), Step(2), Calls(2), call b.
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 5, "headers: {headers:?}");
    assert_eq!(headers[1].header_line_idx, headers[0].header_line_idx + 1);
    assert_eq!(headers[2].header_line_idx, headers[0].header_line_idx + 2);
    assert_eq!(headers[4].header_line_idx, headers[0].header_line_idx + 6);
}

/// LIVE REGION detachment: an armed group renders NOTHING in the
/// transcript — not the group row, not a blank, no matter what its steps
/// hold (expanded output, running calls). The current step streams in the
/// separate TURN LIVE REGION (own layout area), which never enters flatten
/// and never adds a target; a settled folded ladder keeps its usual two
/// lines (see the sibling tests).
#[test]
fn live_region_stays_out_of_flatten_and_adds_no_targets() {
    let mut expanded = tool_call("a", 3);
    expanded.expanded = true;
    let mut running = tool_call("b", 0);
    running.elapsed_ms = None; // still executing: spinner span, same line
    let v = view_with(vec![
        ChatBlock::StepGroup {
            steps: vec![
                Step {
                    thinking_raw: String::new(),
                    thinking: Vec::new(),
                    thinking_dirty: false,
                    calls: vec![tool_call("old", 1)],
                    open: false,
                    calls_open: false,
                    sealed: true,
                },
                Step {
                    thinking_raw: String::new(),
                    thinking: vec![Line::from("working")],
                    thinking_dirty: false,
                    calls: vec![expanded, running],
                    open: false,
                    calls_open: false,
                    sealed: false,
                },
            ],
            open: false,
            progress_active: true,
        },
        marker_n(1),
    ]);
    assert_line_accounting_matches(&v);
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert_eq!(
        flat,
        vec![String::new()],
        "the running group contributes ZERO transcript rows (marker only): {flat:?}"
    );
    let live = super::super::step_group::live_lines(&v);
    assert!(
        live.iter().any(|l| l.contains("Step(2)"))
            && live.iter().any(|l| l.contains("working"))
            && live.iter().any(|l| l.contains("a")),
        "the live region streams the current step: {live:?}"
    );
    // Render-only live area over an absent ladder: no targets at all.
    assert!(
        v.tool_call_headers().is_empty(),
        "no transcript rows -> no targets"
    );
}
