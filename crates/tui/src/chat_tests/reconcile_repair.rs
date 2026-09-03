//! Reliable-completion repair (`reconcile_completed_assistant`) contract:
//! the repair targets the Say THIS run opened (`round_assistant_idx`), never
//! "the last Assistant in the flow". When every `TextDelta` of a run is shed
//! by the bounded worker channel, the last Assistant belongs to the PREVIOUS
//! turn — overwriting it collapsed two prompts into one answer.

use super::*;

fn say_texts(v: &ChatView) -> Vec<String> {
    v.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Assistant { raw, .. } => Some(raw.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn reconcile_repairs_this_runs_say_in_place() {
    // Normal path: the Say streamed (possibly partially shed), the reliable
    // completion replaces its text in place — one Say block, full text.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("q1"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("par".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.reconcile_completed_assistant("partial answer repaired");

    assert_eq!(say_texts(&v), ["partial answer repaired"]);
}

#[test]
fn reconcile_after_all_deltas_shed_inserts_a_new_say() {
    // rules/01 regression (TUI #4): run 2's every TextDelta was dropped
    // under backpressure, so at AssistantFinal the last Assistant in the
    // flow is run 1's Say — the old rposition search overwrote it, leaving
    // two prompts with one answer. The run's Say anchor is absent here, so
    // the repair must INSERT a recovered Say instead.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("q1"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("answer one".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.reconcile_completed_assistant("answer one");

    // Second prompt: no TextDelta ever reached the view (shed), only the
    // reliable completion arrives.
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("q2"),
    });
    v.begin_turn();
    v.reconcile_completed_assistant("answer two");

    assert_eq!(
        say_texts(&v),
        ["answer one", "answer two"],
        "both prompts keep their own answer: {:#?}",
        v.blocks
    );
}

#[test]
fn reconcile_repairs_the_last_say_of_a_multi_say_run() {
    // One run, two Says (each closes its sub-turn): the repair targets the
    // run's LAST Say; the earlier Say keeps its streamed text.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("go"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("interim".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    // next round's Say opens a new sub-turn below the first
    v.apply(&SessionEvent::TextDelta("final d".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.reconcile_completed_assistant("final done");

    assert_eq!(say_texts(&v), ["interim", "final done"]);
}

#[test]
fn double_reconcile_without_a_new_say_inserts_once_then_repairs() {
    // The anchor is KEPT by the repair (cleared only at turn admission):
    // a stale duplicate AssistantFinal (defensive) re-repairs the same
    // block idempotently — it must not INSERT a second recovered Say.
    let mut v = ChatView::default();
    v.blocks.push(ChatBlock::User {
        rendered: crate::markdown::render("q"),
    });
    v.begin_turn();
    v.apply(&SessionEvent::TextDelta("streamed".into()));
    v.apply(&SessionEvent::LlmRoundEnd);
    v.reconcile_completed_assistant("repaired");
    v.reconcile_completed_assistant("repaired");

    assert_eq!(say_texts(&v), ["repaired"]);
}
