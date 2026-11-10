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
    // The FIRST (and only) step is in flight: the ladder renders NOTHING
    // (zero settled steps) and the TURN LIVE REGION is the turn's whole
    // inline surface — region header + calls aggregate + the running
    // call's row, then the marker.
    assert_eq!(v.flatten().len(), 3 + 1, "flat: {:?}", v.flatten());
    let rows: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert_eq!(rows[0], "> streaming", "region header leads: {rows:?}");
    assert!(
        rows.iter().any(|r| r.contains("1 Function call")),
        "the calls aggregate row renders in the region: {rows:?}"
    );
    // The spinner rides ON the running call's row (a SPAN, never an extra
    // line) — same accounting as an ended call.
    assert!(
        rows.iter()
            .any(|r| r.contains("bash") && r.contains("\u{280b}")),
        "the running call's row carries the spinner inline: {rows:?}"
    );
}

/// LIVE FOCUS alignment: a USER-OPENED ladder renders EVERY step row — each
/// finished step one collapsed row, the running step streaming its thinking
/// header/body plus the calls aggregation and call headers — and
/// `collect_headers` must count the same rows so hit-rects stay aligned.
/// The TURN LIVE REGION is a separate area and never enters flatten.
#[test]
fn auto_running_position_keeps_alignment() {
    let v = view_with(vec![
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
    // The ladder renders its SETTLED history (group row + Step(1) row) and
    // the in-flight step streams in the region rows below it.
    assert!(
        flat.iter().any(|l| l.contains("Step(1)")),
        "finished step row renders: {flat:?}"
    );
    assert!(
        flat.iter().all(|l| !l.contains("Step(2)")),
        "the in-flight step never renders as a ladder row (it streams in the \
region below): {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("working")),
        "running thinking streams in the INLINE region rows: {flat:?}"
    );
    // Ladder (group row + Step(1) row + blank = 3) + INLINE region
    // (header + thinking hdr + body + calls aggregate + call row = 5)
    // + marker(1) → 9.
    assert_eq!(v.flatten().len(), 9);
    // Settled ladder targets in visual order: group, Step(1) (the in-flight
    // step adds no ladder target — its surface is the region's own header).
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 2, "headers: {headers:?}");
    assert_eq!(headers[1].header_line_idx, headers[0].header_line_idx + 1);
    assert_eq!(v.live_headers().len(), 1, "one live-region header target");
    assert_eq!(
        v.live_headers()[0].header_line_idx,
        headers[0].header_line_idx + 3,
        "region header sits below group row + step row + blank"
    );
}

/// LIVE REGION INLINE: a folded-but-running ladder keeps its two lines
/// (group row + blank) and the CURRENT step streams in the TURN LIVE
/// REGION whose rows ride INLINE directly below the ladder (inside the
/// same flatten window, above later blocks). The group row counts SETTLED
/// steps only and stays the ladder's only click target; the region adds
/// exactly one target of its own — its `> streaming` header row.
#[test]
fn live_region_rides_inline_below_the_ladder() {
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
    assert!(
        !flat.iter().any(|l| l.contains("old")),
        "the finished step stays folded inside the ladder: {flat:?}"
    );
    assert!(
        flat[0].starts_with("\u{25b8} 1 Step") && flat[0].contains("running"),
        "the group row counts SETTLED steps only, with the live hint: {:?}",
        flat[0]
    );
    assert!(
        flat.iter().all(|l| !l.contains("Step(")),
        "the in-flight step never renders as a ladder row: {flat:?}"
    );
    assert_eq!(
        flat[2], "> streaming",
        "region header right below the blank: {flat:?}"
    );
    assert!(
        flat.iter().any(|l| l.contains("working")) && flat.iter().any(|l| l.contains("a")),
        "the region's rows ride INLINE in flatten: {flat:?}"
    );
    // Folded ladder (2) + region (header + thinking hdr + body + calls
    // aggregate + 2 call rows = 6) + marker (1) = 9; `a`'s expanded result
    // NEVER renders in the region (the ladder owns the drill-down).
    assert_eq!(flat.len(), 9, "flat: {flat:?}");
    let live = super::super::step_group::live_lines(&v);
    assert!(
        live.iter().any(|l| l.contains("working"))
            && live.iter().any(|l| l.contains("a"))
            && live.iter().all(|l| !l.contains("Step(")),
        "the live region streams the current step's rows, counter-less: {live:?}"
    );
    // Folded block only: the ladder keeps exactly ONE target — its group
    // row; the region contributes its own single header target.
    let headers = v.tool_call_headers();
    assert_eq!(headers.len(), 1, "headers: {headers:?}");
    let row = &flat[headers[0].header_line_idx];
    assert!(
        row.contains("1 Step"),
        "target lands on the group row: {row:?}"
    );
    let live_headers = v.live_headers();
    assert_eq!(live_headers.len(), 1, "one live header target");
    assert_eq!(
        live_headers[0].header_line_idx, 2,
        "it points at the rendered `> streaming` row (below group row + blank)"
    );
}
