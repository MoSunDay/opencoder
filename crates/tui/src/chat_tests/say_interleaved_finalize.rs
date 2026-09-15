//! Deterministic think-after-text interleaving: a provider that resumes
//! reasoning BELOW a landed Say opens the next turn's ladder there, so the
//! following TextDelta opens ANOTHER Say — `think -> text` iterating K
//! times strands K open Says. Contract under test: the turn's terminal
//! events (`LlmRoundEnd` + `Done`, no extra repair calls) must leave ZERO
//! open Says, and every merged `Say(n steps)` body must render markdown
//! (no raw `**` markers survive in the flattened transcript).

use super::*;

fn flat_lines(v: &ChatView) -> Vec<String> {
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

fn open_says(v: &ChatView) -> Vec<usize> {
    v.blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::Assistant { done: false, .. } => Some(i),
            _ => None,
        })
        .collect()
}

/// K=4 interleaved pairs in the FINAL round: pre-fix, the round's three
/// repair passes (LlmRoundEnd / Done / TurnDone) each sealed only the LAST
/// open Say, so Says 1-2 stayed `done:false` and their bodies rendered raw
/// `**bold**` under the `Say(n steps)` merged header forever.
#[test]
fn interleaved_k4_round_leaves_no_open_say_after_done() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    for k in 0..4 {
        v.apply(&SessionEvent::ReasoningDelta(format!("think{k} ")));
        v.apply(&SessionEvent::TextDelta(format!(
            "say{k} **bold{k}** tail{k}\n"
        )));
    }
    v.apply(&SessionEvent::LlmRoundEnd);
    v.apply(&SessionEvent::Done);
    // Deliberately NO extra finalize call: the round's own terminal events
    // must suffice (the TurnDone safety net only papers over the leak).
    let open = open_says(&v);
    assert!(
        open.is_empty(),
        "open Says at {open:?} after Done: {:#?}",
        v.blocks
    );

    let flat = flat_lines(&v);
    assert!(
        !flat.iter().any(|l| l.contains("**")),
        "raw markdown markers leaked into flatten: {flat:?}"
    );
    // Merged-header shape: one `Say(1 step)` pair per interleaved batch
    // (each resumed reasoning run folds into its own call-less step), the
    // single-line body is deduped away under the header, exactly one blank
    // between pairs.
    assert_eq!(
        flat,
        vec![
            "▸ Say(1 step): say0 bold0 tail0",
            "",
            "▸ Say(1 step): say1 bold1 tail1",
            "",
            "▸ Say(1 step): say2 bold2 tail2",
            "",
            "▸ Say(1 step): say3 bold3 tail3",
            "",
        ],
        "per-pair headers, markdown-eaten markers, one blank per pair: {flat:?}"
    );
    super::line_accounting::assert_line_accounting_matches(&v);
}

/// Same poison across a TOOL round: tools between the interleaved batches
/// make the ladders carry real calls (`Say(n steps)` counts grow), and the
/// round ends tool-final — the trailing Say must still close and every
/// earlier stranded Say must be sealed.
#[test]
fn interleaved_tool_round_seals_every_stranded_say() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    for k in 0..5 {
        v.apply(&SessionEvent::ReasoningDelta(format!("think{k} ")));
        v.apply(&SessionEvent::TextDelta(format!("say{k} **b{k}**\n")));
        v.apply(&SessionEvent::ToolStart {
            id: format!("t{k}"),
            name: "read".into(),
            input: serde_json::json!({}),
        });
        v.apply(&SessionEvent::ToolEnd {
            id: format!("t{k}"),
            name: "read".into(),
            output: "ok".into(),
            is_error: false,
            images: Vec::new(),
        });
    }
    // Tool-final round: text, then resumed thinking below the Say, then the
    // round ends WITHOUT another Say closing the last ladder.
    v.apply(&SessionEvent::ReasoningDelta("trailing think ".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.apply(&SessionEvent::Done);

    let open = open_says(&v);
    assert!(
        open.is_empty(),
        "open Says at {open:?} after Done: {:#?}",
        v.blocks
    );
    let flat = flat_lines(&v);
    assert!(
        !flat.iter().any(|l| l.contains("**")),
        "raw markdown markers leaked into flatten: {flat:?}"
    );
    // Per-sub-turn step counts stay 1/2/3/4/5 (own ladder only), and the
    // tool-final round's Say bodies dedupe under their headers.
    let headers: Vec<String> = flat
        .iter()
        .filter(|l| l.starts_with("▸ Say("))
        .cloned()
        .collect();
    // Per-sub-turn counting: sub-turn 1's ladder holds only its call-less
    // think step (1); every later ladder holds the sub-turn's own tool plus
    // the resumed think folded as a second call-less step (2 each) — never
    // the run's accumulated total. The tool-final round's last ladder shows
    // its own collapsed row after the final Say.
    assert_eq!(
        headers,
        vec![
            "▸ Say(1 step): say0 b0",
            "▸ Say(2 steps): say1 b1",
            "▸ Say(2 steps): say2 b2",
            "▸ Say(2 steps): say3 b3",
            "▸ Say(2 steps): say4 b4",
        ],
        "per-sub-turn counts 1/2/2/2/2, no accumulation: {flat:?}"
    );
    assert_eq!(flat.last().map(String::as_str), Some(""), "trailing blank");
    assert!(
        flat.contains(&"▸ 2 Steps".to_string()),
        "tool-final round keeps its own collapsed ladder row: {flat:?}"
    );
    assert!(
        !flat
            .iter()
            .any(|l| l.contains("steps): say") && l.contains("**")),
        "headers must carry the RENDERED preview, not raw markdown: {flat:?}"
    );
    super::line_accounting::assert_line_accounting_matches(&v);
}

/// The old Say is sealed the moment a NEW Say opens (K pinned to 1): even
/// mid-round — before any terminal event — at most one open Assistant may
/// exist, so the visible transcript already renders markdown for every
/// landed pair.
#[test]
fn new_say_open_seals_the_previous_one_immediately() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    v.apply(&SessionEvent::TextDelta("first **raw**\n".into()));
    v.apply(&SessionEvent::ReasoningDelta("think after text ".into()));
    // The next TextDelta opens a NEW Say: the first one is now sealed.
    v.apply(&SessionEvent::TextDelta("second **raw**\n".into()));
    let open = open_says(&v);
    assert_eq!(
        open.len(),
        1,
        "exactly the newly-opened Say may stay open mid-round: {:#?}",
        v.blocks
    );
    assert!(
        matches!(v.blocks[open[0]], ChatBlock::Assistant { done: false, .. }),
        "the one open block must be the NEW Say"
    );
    // The sealed first Say renders markdown in flatten even mid-round; the
    // STILL-OPEN second Say legitimately streams raw (the flatten contract
    // renders raw until done) — it gets sealed at the round's end.
    let flat = flat_lines(&v);
    assert!(
        flat.iter().any(|l| l.contains("first raw")),
        "sealed Say body must render (no ** markers): {flat:?}"
    );
    assert!(
        !flat
            .iter()
            .filter(|l| !l.contains("second"))
            .any(|l| l.contains("**raw**")),
        "sealed Say must not show raw markers: {flat:?}"
    );
    v.apply(&SessionEvent::LlmRoundEnd);
    v.apply(&SessionEvent::Done);
    assert!(
        open_says(&v).is_empty(),
        "round end seals the remaining open Say"
    );
    assert!(
        !flat_lines(&v).iter().any(|l| l.contains("**raw**")),
        "after Done no raw markers may survive: {flat:?}"
    );
}
