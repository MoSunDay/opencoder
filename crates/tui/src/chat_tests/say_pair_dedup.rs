//! 合并对（`StepGroup` + 相邻 Say）的正文去重与空行纪律：头部行
//! `Say(n steps): <preview>` 之后必须空出一行再接正文；preview 与正文
//! 同口径（done = 渲染后首行、流式 = raw 首行），正文首个非空行与
//! preview trim 相等时不再重复输出（单行 Say 整块隐藏）。这里逐组合
//! 钉死「恰好一个尾部空行」不变量：多行正文 / 单行正文 / 空正文 /
//! 首行不等（不跳过）/ 前导空行 / 展开态，以及「每子轮计数不累加」契约。

use super::super::*;

fn call_tool(v: &mut ChatView, id: &str) {
    v.apply(&SessionEvent::ToolStart {
        id: id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: id.into(),
        name: "bash".into(),
        output: format!("{id}-out"),
        is_error: false,
        images: Vec::new(),
    });
}

fn lines(v: &ChatView) -> Vec<String> {
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

/// Markdown-first-line contract: the done preview is the RENDERED first
/// line (no raw `#`/`**`/`-` markers in the header) and the body dedup
/// compares against that same rendered text — the heading line is NOT
/// repeated below the header in rendered form, only markdown's own
/// separator blank and the rest of the body render.
#[test]
fn merged_pair_renders_markdown_preview_and_skips_it_in_body() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("# heading\nbody line".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): heading",
            "",
            "    ",
            "    body line",
            "",
        ],
        "rendered preview + body skips that line: {flat:?}"
    );
}

/// A plain (markdown-free) first line still skips in the body — plain text
/// renders to itself, so the done preview equals the first body row and the
/// pair shows the header plus the remaining lines. A single-line Say skips
/// to an all-blank rest and hides the whole body (the header IS the pair).
/// The Full branch (first rendered row differs from the preview) is now
/// unreachable through the pipeline — see the direct unit test below.
#[test]
fn plain_first_line_still_skips_in_body() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("alpha\nbeta".into()));
    v.apply(&SessionEvent::Done);
    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(1 step): alpha", "", "    beta", "",],
    );

    // Single-line plain Say: after skipping the preview line the rest is
    // all blank -> the whole body hides, the header row is the entire pair.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("same".into()));
    v.apply(&SessionEvent::Done);
    assert!(lines(&v).contains(&"\u{25b8} Say(1 step): same".to_string()));
}

/// Defensive coverage for the Full branch of `merged_say_body`: a preview
/// that differs from every row keeps the FULL body rendered. Structurally
/// unreachable through `merged_say_body_decision` since the preview/rendered
/// unification — the done preview IS the first rendered non-empty row (same
/// `line_text` source), so trim-equality holds by construction; this pins
/// the pure function's contract for any future caller passing a foreign
/// preview (e.g. a truncated one).
#[test]
fn merged_say_body_full_when_preview_differs_from_every_row() {
    use super::super::step_render::{merged_say_body, SayBody};
    assert_eq!(
        merged_say_body("preview", &["alpha", "beta"]),
        SayBody::Full
    );
    // Leading blank rows only precede the compared first non-empty row.
    assert_eq!(
        merged_say_body("preview", &["", "  ", "alpha"]),
        SayBody::Full
    );
    // "Differs" is judged AFTER trim: whitespace-only inequality is equality
    // -> Skip, not Full.
    assert_eq!(
        merged_say_body("alpha", &["  alpha  ", "beta"]),
        SayBody::Skip(1)
    );
}

/// Body-dedup combination: an EMPTY Say body hides the whole body block —
/// the header's separator blank IS the pair's single trailing blank, and the
/// Done boundary must not stack a second one.
#[test]
fn merged_pair_empty_body_renders_header_plus_one_blank() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta(String::new()));
    v.apply(&SessionEvent::Done);

    assert_eq!(
        lines(&v),
        vec!["\u{25b8} Say(1 step): ", ""],
        "empty body: header + exactly one blank (no double blank)"
    );
}

/// Body-dedup combination: leading blank rows above the preview line fold
/// into the skip — only the lines BELOW the duplicated first line render.
#[test]
fn merged_pair_skip_folds_leading_blanks_above_the_preview_line() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("\nanswer\nsecond".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    assert_eq!(
        flat,
        vec!["\u{25b8} Say(1 step): answer", "", "    second", ""],
        "skip covers the preview line AND its leading blanks: {flat:?}"
    );
}

/// Open-pair blank discipline with a MULTI-LINE body: header blank, ladder,
/// the ladder's own trailing blank, the deduped body, then exactly one
/// boundary blank — no doubled blanks anywhere in the pair.
#[test]
fn open_pair_multi_line_body_keeps_one_trailing_blank() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("answer\nsecond".into()));
    v.apply(&SessionEvent::Done);
    let headers = v.tool_call_headers();
    v.toggle_tool_call_at(headers[0].block_idx, headers[0].call_idx); // open the ladder

    let flat = lines(&v);
    assert_eq!(flat[0], "\u{276f} Say(1 step): answer");
    assert_eq!(flat[1], "", "ONE blank right after the header: {flat:?}");
    assert!(flat.iter().any(|l| l.contains("Step(1)")), "{flat:?}");
    // Ladder trailing blank + deduped body + exactly one boundary blank.
    assert_eq!(
        &flat[flat.len() - 3..],
        &["", "    second", ""][..],
        "{flat:?}"
    );
    assert!(
        flat[..flat.len() - 1]
            .windows(2)
            .all(|w| !(w[0].trim().is_empty() && w[1].trim().is_empty())),
        "no doubled blanks inside the open pair: {flat:?}"
    );
}

/// Counting contract: ONE run with interleaved text+tool sub-turns renders
/// one merged header per sub-turn, each counting ONLY that sub-turn's own
/// steps — never the accumulated total of the run so far.
#[test]
fn mixed_sub_turns_count_their_own_steps_not_the_run_total() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("r1".into()));
    v.apply(&SessionEvent::TextDelta("first say".into()));
    call_tool(&mut v, "a");
    v.apply(&SessionEvent::ReasoningDelta("r2".into()));
    v.apply(&SessionEvent::TextDelta("second say".into()));
    call_tool(&mut v, "b");
    v.apply(&SessionEvent::TextDelta("third say".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    // Sub-turn 1: the pre-Say reasoning folds into ONE call-less step.
    // Sub-turn 2: tool `a` is step 1, the post-call reasoning folds into a
    // second call-less step. Sub-turn 3: tool `b` alone. Cumulative counting
    // would read 1/3/4 — the exact vec pins the per-sub-turn 1/2/1 AND the
    // one-blank-per-pair discipline.
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): first say",
            "",
            "\u{25b8} Say(2 steps): second say",
            "",
            "\u{25b8} Say(1 step): third say",
            "",
        ],
        "per-sub-turn counts 1/2/1, one blank between pairs: {flat:?}"
    );
    assert!(
        !flat
            .iter()
            .any(|l| l.contains("Say(3 steps)") || l.contains("Say(4 steps)")),
        "counts must not accumulate across sub-turns: {flat:?}"
    );
    super::line_accounting::assert_line_accounting_matches(&v);
}
