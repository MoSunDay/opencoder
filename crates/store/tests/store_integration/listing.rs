//! list_sessions contracts: cursor pagination, search filter, subagent
//! visibility and skill-body surfacing.

use crate::common::{conv, fresh, make_session};
use opencoder_store::{SessionFilter, SessionMeta, Store};

#[tokio::test]
async fn list_preview_preserves_unicode_limits_and_malformed_json_behavior() {
    let (_dir, store) = fresh().await;
    let cases = [
        (
            "short",
            serde_json::json!([{"kind": "text", "text": "sample".repeat(400)}]).to_string(),
            "sample".repeat(14).chars().take(80).collect::<String>(),
        ),
        (
            "unicode",
            serde_json::json!([{"kind": "text", "text": "界".repeat(3000)}]).to_string(),
            "界".repeat(80),
        ),
        (
            "long",
            serde_json::json!([{"kind": "text", "text": "x".repeat(9000)}]).to_string(),
            String::new(),
        ),
        ("malformed", "{broken".into(), String::new()),
        (
            "nul-after-json",
            "[{\"kind\":\"text\",\"text\":\"before\"}]\0ignored".into(),
            "before".into(),
        ),
        (
            "nul-in-json",
            "[{\"kind\":\"text\",\"text\":\"before\0after\"}]".into(),
            String::new(),
        ),
    ];
    for (id, raw, _) in &cases {
        make_session(&store, id, 1).await;
        store
            .append_message(id, &opencoder_core::Message::user(*id, "placeholder"))
            .await
            .unwrap();
        store
            .conn()
            .await
            .unwrap()
            .execute(
                "UPDATE messages SET blocks_json = ? WHERE session_id = ?",
                turso::params![raw.as_str(), *id],
            )
            .await
            .unwrap();
    }
    make_session(&store, "empty", 1).await;
    let listed = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), cases.len() + 1);
    for (id, _, expected) in cases {
        assert_eq!(
            listed.iter().find(|row| row.id == id).unwrap().preview,
            expected,
            "{id}"
        );
    }
    assert!(listed
        .iter()
        .find(|row| row.id == "empty")
        .unwrap()
        .preview
        .is_empty());
}

#[tokio::test]
async fn list_pagination_with_metadata() {
    let (_dir, store) = fresh().await;
    for i in 0..6u32 {
        let id = format!("p{i}");
        make_session(&store, &id, 1000 + i as i64).await;
        store.append_messages(&id, &conv(&id, 1)).await.unwrap();
    }

    let page1 = store
        .list_sessions(&SessionFilter {
            limit: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);
    // newest first
    assert_eq!(page1[0].id, "p5");
    assert_eq!(page1[1].id, "p4");
    assert!(page1[0].preview.contains("p5 msg 0"));

    let cursor = format!("{}|{}", page1[2].created_at, page1[2].id);
    let page2 = store
        .list_sessions(&SessionFilter {
            limit: 3,
            cursor: Some(cursor),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].id, "p2");

    let hits = store
        .list_sessions(&SessionFilter {
            limit: 10,
            search: Some("p3".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "p3");
}

#[tokio::test]
async fn list_sessions_excludes_subagents_by_default() {
    use opencoder_store::{SubagentStatus, SubagentTaskRecord};

    let (_dir, store) = fresh().await;
    // Parent and child sessions.
    make_session(&store, "parent", 1000).await;
    make_session(&store, "child-sub", 2000).await;

    // Link child as a subagent of parent.
    let rec = SubagentTaskRecord {
        task_id: "task-1".into(),
        parent_session_id: "parent".into(),
        child_session_id: "child-sub".into(),
        parent_message_id: None,
        agent: "explore".into(),
        prompt: "do stuff".into(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 1500,
        completed_at: None,
    };
    store.create_subagent_task(&rec).await.unwrap();

    // Default filter (include_subagents == false) excludes the child.
    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    assert_eq!(
        items.len(),
        1,
        "subagent session should be excluded by default"
    );
    assert_eq!(items[0].id, "parent");

    // With include_subagents == true, both appear.
    let filter = SessionFilter {
        include_subagents: true,
        ..Default::default()
    };
    let items = store.list_sessions(&filter).await.unwrap();
    assert_eq!(
        items.len(),
        2,
        "both parent and child should appear with include_subagents"
    );
}

#[tokio::test]
async fn list_sessions_carries_skill_body_for_picker_tag() {
    let (_dir, store) = fresh().await;
    // Session with an active skill: the store persists the body, not the name.
    store
        .create_session(&SessionMeta {
            id: "skilled".into(),
            agent: Some("plan".into()),

            autopilot_mode: None,
            skill: Some("## do-and-done\nfull body".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .create_session(&SessionMeta {
            id: "plain".into(),
            agent: Some("act".into()),

            autopilot_mode: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let items = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap();
    let skilled = items
        .iter()
        .find(|s| s.id == "skilled")
        .expect("skilled row");
    assert_eq!(
        skilled.skill.as_deref(),
        Some("## do-and-done\nfull body"),
        "list() must surface the stored skill body so the /task picker can tag it"
    );
    let plain = items.iter().find(|s| s.id == "plain").expect("plain row");
    assert_eq!(plain.skill, None, "sessions without a skill must list None");
}
