//! 合并对（`StepGroup` + 相邻 Say）的正文去重与空行纪律：头部行
//! `Say(n steps): <preview>` 之后必须空出一行再接正文；正文首个非空行
//! 与 preview trim 相等时不再重复输出（单行 Say 整块隐藏）。这里逐组合
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

/// Body-dedup combination: when the Say's first RENDERED line does not trim-
/// equal the raw preview (a markdown heading renders without its `#`), the
/// body stays FULL — the skip is exact-equality only, never prefix matching.
#[test]
fn merged_pair_keeps_full_body_when_first_line_differs_from_preview() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think".into()));
    call_tool(&mut v, "t1");
    v.apply(&SessionEvent::TextDelta("# heading\nbody line".into()));
    v.apply(&SessionEvent::Done);

    let flat = lines(&v);
    // The preview keeps the RAW first line (`# heading`); the rendered body
    // starts with the styled heading text (plus markdown's own separator) —
    // not trim-equal, so nothing is skipped and the body renders in full
    // below the header's separator blank.
    assert_eq!(
        flat,
        vec![
            "\u{25b8} Say(1 step): # heading",
            "",
            "    heading",
            "    ",
            "    body line",
            "",
        ],
        "no trim-equal first line -> no skip: {flat:?}"
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
