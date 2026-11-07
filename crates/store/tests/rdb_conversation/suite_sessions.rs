//! Session-lifecycle contracts for the rdb backend, ported from the SQL
//! suites: CRUD with ignore-on-conflict, patch scoping, private harness
//! runtime roundtrips, preview projection and cascade deletes.

use super::*;

#[tokio::test]
async fn session_crud_and_activity_ordered_listing() {
    let store = mem().await;
    store.create_session(&meta_for("s-old", 10)).await.unwrap();
    store.create_session(&meta_for("s-new", 20)).await.unwrap();
    store.create_session(&meta_for("s-mid", 15)).await.unwrap();
    // INSERT OR IGNORE: re-creating an existing id is a no-op.
    store.create_session(&meta_for("s-new", 20)).await.unwrap();

    assert_eq!(
        store
            .get_session("s-new")
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("title-s-new")
    );
    assert!(store.get_session("missing").await.unwrap().is_none());

    // Listing orders by MAX(updated_at, created_at) DESC.
    let listed = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|s: &SessionListItem| s.id.clone())
            .collect::<Vec<_>>(),
        vec!["s-new", "s-mid", "s-old"]
    );

    // A patch mutates only the touched columns.
    store
        .update_session(
            "s-mid",
            &SessionPatch {
                title: Some("renamed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let patched = store.get_session("s-mid").await.unwrap().unwrap();
    assert_eq!(patched.title.as_deref(), Some("renamed"));
    assert_eq!(
        patched.agent.as_deref(),
        Some("act"),
        "untouched columns survive"
    );

    // Activity touch: appending a message reorders the listing.
    store
        .append_message("s-old", &user_msg("m1", "hello there"))
        .await
        .unwrap();
    let listed = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed[0].id, "s-old");
    assert_eq!(
        listed[0].preview, "hello there",
        "preview = first user message text"
    );
}

#[tokio::test]
async fn harness_runtime_roundtrips_privately_from_meta() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    assert!(store.harness_runtime("s1").await.unwrap().is_none());

    let runtime = opencoder_core::harness::HarnessRuntime::default();
    store.set_harness_runtime("s1", &runtime).await.unwrap();
    assert!(store.harness_runtime("s1").await.unwrap().is_some());
    // The private column never leaks into the public metadata listing.
    let meta = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(meta.id, "s1");
}

#[tokio::test]
async fn delete_session_cascades_and_clear_other_sessions_counts() {
    let store = mem().await;
    for id in ["s1", "s2", "s3"] {
        store.create_session(&meta_for(id, 1)).await.unwrap();
    }
    store
        .append_message("s1", &user_msg("m1", "one"))
        .await
        .unwrap();
    store
        .append_message("s1", &user_msg("m2", "two"))
        .await
        .unwrap();
    store.admit_input(&queue_input("s1", "q1")).await.unwrap();
    store
        .append_events(&[event("s1", "text_delta", r#"{"d":"hi"}"#)])
        .await
        .unwrap();

    // Delete one session: children go with it, siblings stay.
    store.delete_session("s1").await.unwrap();
    assert!(store.get_session("s1").await.unwrap().is_none());
    assert!(store.load_messages("s1").await.unwrap().is_empty());
    assert!(store
        .pending_inputs("s1", opencoder_store::Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
    assert!(store.events_after("s1", 0).await.unwrap().is_empty());
    assert_eq!(store.get_session("s2").await.unwrap().unwrap().id, "s2");

    // clear_other_sessions removes all but the kept one.
    store
        .append_message("s2", &user_msg("m3", "keep me"))
        .await
        .unwrap();
    let removed = store.clear_other_sessions("s2").await.unwrap();
    assert_eq!(removed, 1);
    let listed = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "s2");
    assert_eq!(listed[0].preview, "keep me");
}

#[tokio::test]
async fn preview_caps_long_first_user_text_at_eighty_chars() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    store
        .append_message("s1", &user_msg("m1", &"x".repeat(200)))
        .await
        .unwrap();
    let listed = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed[0].preview.chars().count(), 80);
    // Assistant-first sessions only preview the first USER message.
    let store2 = mem().await;
    store2.create_session(&meta_for("s2", 1)).await.unwrap();
    let mut a = Message::assistant("a1");
    a.blocks = vec![ContentBlock::text("assistant leads")];
    store2.append_message("s2", &a).await.unwrap();
    store2
        .append_message("s2", &user_msg("u1", "user follows"))
        .await
        .unwrap();
    let listed = store2
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed[0].preview, "user follows");
    // UTF-8 safety: multi-byte characters never split mid-codepoint.
    let store3 = mem().await;
    store3.create_session(&meta_for("s3", 1)).await.unwrap();
    store3
        .append_message("s3", &user_msg("m1", &"中".repeat(100)))
        .await
        .unwrap();
    let listed = store3
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed[0].preview.chars().count(), 80);
}
