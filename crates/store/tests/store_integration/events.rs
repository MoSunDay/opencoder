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
