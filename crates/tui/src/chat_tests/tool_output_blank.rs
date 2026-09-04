//! Tool-output trailing-blank discipline (`User:`-block parity): after an
//! expanded call's result there is EXACTLY ONE blank line — never doubled by
//! the output's own trailing newlines (capture trims them) nor by the group's
//! trailing blank (the final call's separator merges into it).

use super::super::*;
use ratatui::text::{Line, Span};

use super::line_accounting::assert_line_accounting_matches;

fn out_row(text: &str) -> Line<'static> {
    Line::from(Span::raw(format!("  {text}")))
}

fn expanded_call(id: &str, out_lines: usize) -> ToolCall {
    ToolCall {
        id: id.into(),
        header: Line::from(Span::raw(format!("\u{25b8} {id}"))),
        output: (0..out_lines).map(|_| out_row("out")).collect(),
        started_at_ms: Some(0),
        elapsed_ms: Some(0),
        expanded: true,
    }
}

fn collapsed_call(id: &str) -> ToolCall {
    ToolCall {
        expanded: false,
        ..expanded_call(id, 0)
    }
}

fn one_step_group(calls: Vec<ToolCall>) -> ChatBlock {
    ChatBlock::StepGroup {
        steps: vec![Step {
            thinking_raw: String::new(),
            thinking: Vec::new(),
            thinking_dirty: false,
            calls,
            open: true,
            calls_open: true,
            sealed: true,
        }],
        open: true,
        progress_active: false,
    }
}

fn rows(view: &ChatView) -> Vec<String> {
    view.flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect()
}

fn assert_no_consecutive_blank(view: &ChatView) {
    let r = rows(view);
    for w in r.windows(2) {
        assert!(
            !(w[0].trim().is_empty() && w[1].trim().is_empty()),
            "consecutive blank rows in {r:?}"
        );
    }
}

#[test]
fn final_expanded_call_result_ends_with_exactly_one_blank() {
    // group(1) + step(1) + calls(1) + hdr(1) + out(2) + ONE blank = 7.
    // The old shape pushed the per-call separator AND the group trailing
    // blank — two blanks after the result.
    let v = ChatView {
        blocks: vec![one_step_group(vec![expanded_call("a", 2)])],
        ..Default::default()
    };
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
    let r = rows(&v);
    assert_eq!(r.len(), 7, "no doubled separator: {r:?}");
    assert!(r[5].contains("out"));
    assert_eq!(r[6], "", "exactly one trailing blank after the result");
}

#[test]
fn non_final_expanded_call_keeps_its_separator() {
    // a expanded (2 out rows) + b collapsed: a's separator must stay so the
    // collapsed row below does not stick to the result — one blank, no more.
    let v = ChatView {
        blocks: vec![one_step_group(vec![
            expanded_call("a", 2),
            collapsed_call("b"),
        ])],
        ..Default::default()
    };
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
    let r = rows(&v);
    // group(1) + step(1) + calls(1) + a-hdr(1) + a-out(2) + sep(1) + b-hdr(1)
    // + trailing blank(1) = 9.
    assert_eq!(r.len(), 9, "{r:?}");
    assert_eq!(r[6], "", "single separator between result and next call");
    assert!(r[7].contains("b"));
}

#[test]
fn every_call_expanded_still_yields_single_blanks() {
    let v = ChatView {
        blocks: vec![one_step_group(vec![
            expanded_call("a", 1),
            expanded_call("b", 1),
        ])],
        ..Default::default()
    };
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
    // group+step+calls + a-hdr+a-out+sep + b-hdr+b-out + trailing = 9.
    assert_eq!(rows(&v).len(), 9);
}

#[test]
fn tool_end_capture_drops_trailing_blank_lines() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo hi"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "hi\n\n\n".into(),
        is_error: false,
        images: Vec::new(),
    });
    if let Some(ChatBlock::StepGroup { steps, .. }) = v.blocks.first() {
        let c = &steps[0].calls[0];
        assert_eq!(
            c.output.len(),
            1,
            "trailing blank rows trimmed: {:?}",
            c.output
                .iter()
                .map(|l| l
                    .spans
                    .iter()
                    .map(|s| s.content.clone())
                    .collect::<String>())
                .collect::<Vec<_>>()
        );
    } else {
        panic!("expected a StepGroup");
    }

    // Expanding the finished call renders exactly one trailing blank.
    for idx in 0..=3 {
        v.toggle_tool_call_at(0, idx);
    }
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
}

#[test]
fn interior_blank_lines_are_preserved() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "printf 'a\\n\\nb'"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "a".into(),
        name: "bash".into(),
        output: "a\n\nb".into(),
        is_error: false,
        images: Vec::new(),
    });
    for idx in 0..=3 {
        v.toggle_tool_call_at(0, idx);
    }
    if let Some(ChatBlock::StepGroup { steps, .. }) = v.blocks.first() {
        let c = &steps[0].calls[0];
        assert_eq!(c.output.len(), 3, "interior blank kept, trailing trimmed");
    }
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
    let r = rows(&v);
    assert!(
        r.iter().any(|l| l.contains("a")) && r.iter().any(|l| l.contains("b")),
        "both output lines visible: {r:?}"
    );
}

#[test]
fn finish_bash_tool_output_trims_trailing_blanks() {
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    v.finish_bash_tool("hello\n\n");
    assert_line_accounting_matches(&v);
    assert_no_consecutive_blank(&v);
    if let Some(ChatBlock::StepGroup { steps, .. }) = v.blocks.last() {
        let c = &steps[0].calls[0];
        assert_eq!(c.output.len(), 1, "trailing blanks dropped: {c:?}");
    }
    // The whole expanded `!cmd` ladder ends with exactly one blank.
    let r = rows(&v);
    assert_eq!(r.last().map(String::as_str), Some(""));
    assert!(
        !r[r.len() - 2].trim().is_empty(),
        "single trailing blank: {r:?}"
    );
}

#[test]
fn collapsed_group_shape_is_unchanged() {
    // A closed group (the default) still renders exactly group row + blank.
    let v = ChatView {
        blocks: vec![ChatBlock::StepGroup {
            steps: vec![Step {
                thinking_raw: String::new(),
                thinking: Vec::new(),
                thinking_dirty: false,
                calls: vec![collapsed_call("a")],
                open: false,
                calls_open: false,
                sealed: true,
            }],
            open: false,
            progress_active: false,
        }],
        ..Default::default()
    };
    assert_line_accounting_matches(&v);
    // `chat_step_render::flatten_step_group` pluralizes the count (0c2de6c):
    // one step renders "1 Step", several render "N Steps".
    assert_eq!(rows(&v), vec!["\u{25b8} 1 Step", ""]);
}
