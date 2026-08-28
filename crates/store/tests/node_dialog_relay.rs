//! P3 message-relay store contracts:
//! - `load_message_rows` returns the TRUE per-session `seq` + raw blocks JSON
//!   (the resume boundary unit the relay slice selector filters on)
//! - `dispatch_node_task_for_session` binds a task to an EXISTING session
//!   without creating a synthetic one, and refuses unknown sessions.

use opencoder_core::{Message, Role};
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store, TASK_TYPE_NODE};

async fn fresh() -> (tempfile::TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

fn meta(id: &str, task_type: Option<&str>) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        model: None,
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 1,
        updated_at: 1,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: task_type.map(str::to_string),
        requirement: None,
    }
}

/// Relay read model: rows come back in seq order with the stored seq values
/// (not positional guesses) and blocks as the raw JSON structure.
#[tokio::test]
async fn load_message_rows_carries_true_seq_and_raw_blocks() {
    let (_dir, store) = fresh().await;
    store.create_session(&meta("s-rows", None)).await.unwrap();

    let mut user = Message::user("m1", "hello there");
    user.created_at = 10;
    let mut assistant = Message::assistant("m2");
    assistant.blocks = vec![opencoder_core::ContentBlock::text("hi back")];
    assistant.created_at = 20;
    store.append_message("s-rows", &user).await.unwrap();
    store.append_message("s-rows", &assistant).await.unwrap();

    let rows = store.load_message_rows("s-rows").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].seq, 1, "first persisted seq is 1");
    assert_eq!(rows[1].seq, 2);
    assert_eq!(rows[0].role, "user");
    assert_eq!(rows[1].role, "assistant");
    assert_eq!(rows[0].created_at, 10);
    assert_eq!(rows[1].created_at, 20);
    assert_eq!(rows[0].blocks[0]["kind"], "text");
    assert_eq!(rows[0].blocks[0]["text"], "hello there");
    assert_eq!(rows[1].blocks[0]["text"], "hi back");
}

/// An unknown (or message-less) session yields an empty slice, not an error.
#[tokio::test]
async fn load_message_rows_empty_and_unknown_session_are_empty() {
    let (_dir, store) = fresh().await;
    store.create_session(&meta("s-empty", None)).await.unwrap();
    assert!(store.load_message_rows("s-empty").await.unwrap().is_empty());
    assert!(store.load_message_rows("s-ghost").await.unwrap().is_empty());
}

/// `append_message` seqs and `load_message_rows` seqs agree, so a resume
/// slice boundary (summary_seq from session meta) matches real rows.
#[tokio::test]
async fn load_message_rows_seq_matches_last_message_seq() {
    let (_dir, store) = fresh().await;
    store.create_session(&meta("s-seq", None)).await.unwrap();
    for i in 0..3 {
        store
            .append_message("s-seq", &Message::user(format!("m{i}"), "x"))
            .await
            .unwrap();
    }
    let last = store.last_message_seq("s-seq").await.unwrap();
    let rows = store.load_message_rows("s-seq").await.unwrap();
    assert_eq!(rows.last().unwrap().seq, last);
    assert_eq!(rows.len(), 3);
}

/// Session-reuse dispatch: the node_task binds to the existing session and NO
/// synthetic session row appears (the session's own fields stay untouched).
#[tokio::test]
async fn dispatch_for_session_binds_existing_session_without_new_one() {
    let (_dir, store) = fresh().await;
    let node = store
        .register_node("relay-node", Some("v1"), None, None, 100)
        .await
        .unwrap();

    store.create_session(&meta("s-dialog", None)).await.unwrap();
    store
        .append_message("s-dialog", &Message::user("m1", "first question"))
        .await
        .unwrap();
    let before = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();

    let rec = store
        .dispatch_node_task_for_session(
            "t-resume",
            "s-dialog",
            &node.id,
            Some("continue it"),
            "follow-up prompt",
            Some("build"),
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(rec.session_id, "s-dialog");
    assert_eq!(rec.node_id, node.id);
    assert_eq!(rec.status.as_str(), "pending");

    // Exactly the pre-existing session remains, untouched by the dispatch.
    let after = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(after.len(), before.len(), "no synthetic session created");
    let dialog = store.get_session("s-dialog").await.unwrap().unwrap();
    assert_ne!(dialog.task_type.as_deref(), Some(TASK_TYPE_NODE));
    let msgs = store.load_messages("s-dialog").await.unwrap();
    assert_eq!(msgs.len(), 1, "dispatch must not write messages");

    // The task is claimable through the normal queue path.
    let claimed = store.claim_next_node_task(&node.id, 300).await.unwrap();
    assert_eq!(claimed.unwrap().session_id, "s-dialog");
}

/// Binding to a missing session is a hard error (HTTP maps it to 400).
#[tokio::test]
async fn dispatch_for_session_missing_session_errors() {
    let (_dir, store) = fresh().await;
    let node = store
        .register_node("relay-node", None, None, None, 100)
        .await
        .unwrap();
    let err = store
        .dispatch_node_task_for_session(
            "t-ghost",
            "s-missing",
            &node.id,
            None,
            "p",
            None,
            None,
            100,
        )
        .await;
    assert!(err.is_err(), "missing session must be refused");
    assert!(
        store.get_node_task("t-ghost").await.unwrap().is_none(),
        "failed dispatch must not leave a queue row"
    );
}

/// Unknown node is still refused on the reuse path (same contract as the
/// synthetic-session dispatch).
#[tokio::test]
async fn dispatch_for_session_unknown_node_errors() {
    let (_dir, store) = fresh().await;
    store.create_session(&meta("s-here", None)).await.unwrap();
    let err = store
        .dispatch_node_task_for_session("t-x", "s-here", "no-such-node", None, "p", None, None, 1)
        .await;
    assert!(err.is_err());
}

/// Guard the role literal vocabulary the relay protocol freezes: the stored
/// role strings are exactly the four wire roles.
#[tokio::test]
async fn stored_role_literals_match_wire_vocabulary() {
    let (_dir, store) = fresh().await;
    store.create_session(&meta("s-roles", None)).await.unwrap();
    for (id, role) in [
        ("r1", Role::User),
        ("r2", Role::Assistant),
        ("r3", Role::Tool),
        ("r4", Role::System),
    ] {
        let mut m = Message::assistant(id);
        m.role = role;
        store.append_message("s-roles", &m).await.unwrap();
    }
    let roles: Vec<String> = store
        .load_message_rows("s-roles")
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.role)
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "system"]);
}
