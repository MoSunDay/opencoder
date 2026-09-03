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
    // An unfinished call (elapsed_ms == None) adds a spinner SPAN to the
    // group row — never an extra line. Accounting must stay identical.
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
    // group row + trailing blank + marker_n.
    assert_eq!(v.flatten().len(), 2 + 1);
    // The spinner hint rides on the group row itself.
    let row: String = v.flatten()[0]
        .spans
        .iter()
        .map(|s| s.content.clone())
        .collect();
    assert!(
        row.contains("running"),
        "group row should hint running: {row:?}"
    );
    assert!(
        row.contains("Step  \u{280b} running"),
        "group row should keep two spaces before the animation: {row:?}"
    );
}
