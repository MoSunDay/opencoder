//! Regression for the transcript-pollution bug: a handoff boundary must clear
//! residual compaction metadata (`summary_seq`). Compaction computes its skip
//! offset as `prev_skip = summary_seq.or(handoff_seq)` (`compaction.rs`); a
//! stale smaller `summary_seq` left behind by an earlier compaction would win
//! over the newer `handoff_seq`, producing an OFFSET that is too small and
//! re-loading already-summarized messages on the next compaction/resume.
//!
//! Two layers fix this:
//!   - write-time: the handoff persistence path sets `clear_summary: true`.
//!   - resume-time: `resume()` zeroes stale compaction fields when a handoff
//!     exists (defensive, covers pre-fix dirty rows).
//!
//! These tests target the resume-time guard end-to-end, including a real
//! compaction driven after resume to prove the persisted OFFSET is correct.

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::compaction::compact;
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn user(id: &str, text: &str) -> Message {
    Message::user(id, text)
}

fn assistant(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// A compaction-capable mock: one canned summary response reused on demand.
fn summary_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("resumed summary".into()),
        LlmEvent::Completed {
            text: "resumed summary".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 1,
                total_tokens: 2,
                ..Usage::default()
            }),
        },
    ]))
}

/// Dirty store state: a compaction set `summary_seq = 2`, then a handoff set
/// `handoff_seq = 4` WITHOUT clearing the summary (the pre-fix data shape).
/// Resume must zero `summary_seq` and reconstruct the focused post-handoff
/// transcript, never the stale compacted head.
#[tokio::test]
async fn resume_handoff_clears_stale_summary_seq() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Plan-mode head (4 msgs) + post-handoff tail (2 msgs) in the append-only store.
    let msgs = vec![
        user("u1", "plan a feature"),
        assistant("a1", "exploring"),
        user("u2", "approved"),
        assistant("a2", "## Plan\n1. build it"),
        user("u3", "go"),
        assistant("a3", "step 1 done"),
    ];
    store.append_messages("s1", &msgs).await.unwrap();

    // Persist the DIRTY state: stale compaction (summary_seq=2) coexisting with
    // a newer handoff boundary (handoff_seq=4). A fixed handoff path would emit
    // `clear_summary: true`; here we deliberately leave the residue to prove the
    // resume-time guard handles pre-fix data.
    store
        .update_session(
            "s1",
            &opencoder_store::SessionPatch {
                summary: Some("stale summary".into()),
                summary_seq: Some(2),
                summary_images: Some(vec!["stale.png".into()]),
                handoff_seq: Some(4),
                handoff_plan: Some("## Plan\n1. build it".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let resumed = resume(
        store,
        "s1",
        cfg(),
        Arc::new(MockChatClient::new()),
        dir.path().to_path_buf(),
    )
    .await
    .expect("resume must succeed");

    // The core fix: residual compaction metadata is zeroed in the SessionState.
    assert_eq!(
        resumed.summary_seq, None,
        "stale summary_seq must be cleared on handoff resume"
    );
    assert_eq!(resumed.summary, None, "stale summary text must be cleared");
    assert!(
        resumed.summary_images.is_empty(),
        "stale summary_images must be cleared"
    );
    assert_eq!(resumed.handoff_seq, Some(4), "handoff boundary preserved");

    // Loading used the handoff path (full load + trim), not a corrupted OFFSET:
    // [plan_instruction, u3, a3] only -- the plan-mode head is gone.
    assert_eq!(
        resumed.messages.len(),
        3,
        "resumed transcript is plan instruction + tail only"
    );
    assert_eq!(resumed.messages[0].role, Role::User);
    assert!(
        resumed.messages[0].synthetic,
        "first message is the handoff instruction"
    );
    assert!(
        resumed.messages[0].text().contains("## Plan"),
        "plan text present"
    );
    assert_eq!(resumed.messages[1].id, "u3");
    assert_eq!(resumed.messages[2].id, "a3");
}

/// End-to-end OFFSET proof: after resume zeroes `summary_seq`, a subsequent
/// compaction computes `prev_skip = summary_seq.or(handoff_seq) = handoff_seq`,
/// yielding the correct persisted `summary_seq`. With the stale `summary_seq`
/// leaked through (the bug), `prev_skip` would be the smaller stale value and
/// `summary_seq` would be too small -- re-summarized messages would resurface.
#[tokio::test]
async fn clear_summary_prevents_offset_corruption() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "s2".into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // 4-message plan head + 8-message post-handoff tail (12 store msgs).
    let plan_head = vec![
        user("u1", "plan"),
        assistant("a1", "## Plan\n1. build it"),
        user("u2", "yes"),
        assistant("a2", "noted"),
    ];
    let tail = [
        user("u3", "go"),
        assistant("a3", "t1"),
        user("u4", "more"),
        assistant("a4", "t2"),
        user("u5", "more"),
        assistant("a5", "t3"),
        user("u6", "more"),
        assistant("a6", "t4"),
    ];
    let mut all = plan_head.clone();
    all.extend(tail.iter().cloned());
    store.append_messages("s2", &all).await.unwrap();

    let handoff_seq = plan_head.len() as i64; // 4
                                              // Dirty state: stale summary_seq=2 < handoff_seq=4. This is exactly the
                                              // data shape that, pre-fix, made compaction pick the wrong OFFSET.
    store
        .update_session(
            "s2",
            &opencoder_store::SessionPatch {
                summary: Some("stale summary".into()),
                summary_seq: Some(2),
                handoff_seq: Some(handoff_seq),
                handoff_plan: Some("## Plan\n1. build it".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    // Hand the compaction-capable mock to resume (resume makes no LLM call; it
    // is reused by the compact() below).
    let mut resumed = resume(
        store.clone(),
        "s2",
        cfg(),
        summary_client(),
        dir.path().to_path_buf(),
    )
    .await
    .expect("resume must succeed");

    // Precondition established by the resume-time guard.
    assert_eq!(
        resumed.summary_seq, None,
        "stale summary_seq cleared -> compaction will use handoff_seq as prev_skip",
    );
    assert_eq!(resumed.handoff_seq, Some(handoff_seq));

    // The resumed transcript is [plan_instr, u3..a6] (9 msgs). Compaction keeps
    // the last 2 turns (u5,u6) as the tail and summarizes the preceding head
    // (plan_instr + u3,a3,u4,a4). With prev_skip = handoff_seq = 4 and a 4-msg
    // head, new_skip must be 8 -- i.e. every message before the tail is marked
    // summarized. A leaked summary_seq=2 would give prev_skip=2 -> new_skip=6
    // (OFFSET too small; u4,a4 would resurface on the next resume).
    let outcome = compact(&mut resumed, &HashMap::new(), &mut |_| {}).await;
    assert!(outcome.is_ok(), "compaction must succeed: {outcome:?}");

    assert_eq!(
        resumed.summary_seq,
        Some(8),
        "OFFSET must be handoff_seq(4) + head(4) = 8, not the stale 2 + head",
    );

    // The store row reflects the corrected OFFSET.
    let m = store.get_session("s2").await.unwrap().unwrap();
    assert_eq!(
        m.summary_seq,
        Some(8),
        "persisted summary_seq matches the corrected OFFSET"
    );
}
