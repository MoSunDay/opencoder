//! Session event stream, delivery enum parsing and last_message_seq tracking.

use crate::common::{fresh, make_session};
use opencoder_core::Message;
use opencoder_store::Store;

#[tokio::test]
async fn events_append_and_after_replay() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;
    use opencoder_store::{EventKind, SessionEventRecord};
    for i in 0..5u32 {
        store
            .append_event(&SessionEventRecord {
                session_id: "s".into(),
                kind: if i == 0 {
                    EventKind::PromptAdmitted
                } else {
                    EventKind::TextDelta
                },
                payload: serde_json::json!({"i": i}),
                ts: i as i64,
                seq: None,
                sse_kind: None,
            })
            .await
            .unwrap();
    }
    // replay after seq 2 -> events 3,4,5 (3 events, payloads i=2,3,4)
    let tail = store.events_after("s", 2).await.unwrap();
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0].payload["i"], 2);
    assert!(tail[0].seq.unwrap() > 2);
}

#[tokio::test]
async fn backend_name_reports_libsql() {
    let (_dir, store) = fresh().await;
    assert_eq!(store.backend_name(), "libsql");
}

#[tokio::test]
async fn last_message_seq_tracks_appends() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 0).await;
    assert_eq!(store.last_message_seq("s").await.unwrap(), 0);

    let msg1 = Message::user("u1", "hello");
    let seq1 = store.append_message("s", &msg1).await.unwrap();
    assert_eq!(seq1, 1);
    assert_eq!(store.last_message_seq("s").await.unwrap(), 1);

    let msg2 = Message::assistant("u2");
    let seq2 = store.append_message("s", &msg2).await.unwrap();
    assert_eq!(seq2, 2);
    assert_eq!(store.last_message_seq("s").await.unwrap(), 2);
}

#[tokio::test]
async fn delivery_parse_and_as_str_roundtrip() {
    use opencoder_store::Delivery;
    assert_eq!(Delivery::parse("steer"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("queue"), Some(Delivery::Queue));
    assert_eq!(Delivery::parse("invalid"), None);
    assert_eq!(Delivery::Steer.as_str(), "steer");
    assert_eq!(Delivery::Queue.as_str(), "queue");
    // case-insensitive
    assert_eq!(Delivery::parse("STEER"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("Queue"), Some(Delivery::Queue));
    // whitespace-tolerant (a padded " queue " must not degrade to Steer)
    assert_eq!(Delivery::parse("  queue  "), Some(Delivery::Queue));
    assert_eq!(Delivery::parse("\tSTEER\n"), Some(Delivery::Steer));
    assert_eq!(Delivery::parse("   "), None);
    assert_eq!(Delivery::parse(" stear "), None, "a typo must stay invalid");
}

/// `events_of_kinds` (SQL override) must agree with the trait's default
/// filter semantics: only tagged `sse_kind` rows, in seq order, empty kinds
/// → no rows, legacy untagged rows never match.
#[tokio::test]
async fn events_of_kinds_filters_tagged_rows_in_seq_order() {
    let (_dir, store) = fresh().await;
    make_session(&store, "k", 1).await;
    use opencoder_store::{EventKind, SessionEventRecord};
    let row = |kind: &str, ts: i64| SessionEventRecord {
        session_id: "k".into(),
        kind: EventKind::Step,
        payload: serde_json::json!({ "ts": ts }),
        ts,
        seq: None,
        sse_kind: Some(kind.to_string()),
    };
    let legacy = SessionEventRecord {
        session_id: "k".into(),
        kind: EventKind::Step,
        payload: serde_json::json!({ "legacy": true }),
        ts: 99,
        seq: None,
        sse_kind: None,
    };
    store
        .append_events(&[
            row("text_delta", 1),
            row("llm_usage", 2),
            row("subagent_child", 3),
            row("llm_usage", 4),
            legacy,
        ])
        .await
        .unwrap();

    let picked = store
        .events_of_kinds("k", &["llm_usage", "subagent_child"])
        .await
        .unwrap();
    assert_eq!(picked.len(), 3);
    assert_eq!(
        picked
            .iter()
            .map(|r| r.sse_kind.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("llm_usage"), Some("subagent_child"), Some("llm_usage")]
    );
    let seqs: Vec<i64> = picked.iter().map(|r| r.seq.unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "seq order preserved");

    // The default-impl semantics on the same rows: untagged never matches,
    // an empty kind list matches nothing, other sessions stay isolated.
    assert!(store.events_of_kinds("k", &[]).await.unwrap().is_empty());
    assert!(store
        .events_of_kinds("missing", &["llm_usage"])
        .await
        .unwrap()
        .is_empty());
}
