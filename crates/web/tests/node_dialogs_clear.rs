//! DELETE /api/nodes/:id/dialogs — the Operator console's one-click "delete
//! all sessions". HTTP contract over the real router + real store:
//!   * sessions bound to a TERMINAL node task (done | error | cancelled) are
//!     removed with their FK cascades (messages/events/inputs/node_tasks row),
//!   * sessions whose task is still pending/running/cancelling are SKIPPED and
//!     reported in `skipped`,
//!   * unknown node answers 404; an empty sweep answers removed:0.

mod node_e2e_support;
mod support;

use opencoder_store::NodeTaskStatus;

use node_e2e_support::{get_json, post_json, spawn_server, Server, TOKEN};

async fn delete_json(base: &str, path: &str) -> (reqwest::StatusCode, serde_json::Value) {
    let (name, value) = support::auth_header(TOKEN);
    let r = node_e2e_support::http()
        .delete(format!("{base}{path}"))
        .header(name, value)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
    (status, v)
}

/// Server + registered node + three dispatched tasks:
///   A running, B pending, C done — the exact sweep boundary.
async fn setup() -> (Server, String, String, String, String) {
    let srv = spawn_server().await;
    let base = srv.base.clone();
    let store = srv.store.clone();
    let (_, v) = post_json(
        &base,
        "/api/nodes/register",
        Some(serde_json::json!({ "name": "sweep-node" })),
    )
    .await;
    let node_id = v["node_id"].as_str().unwrap().to_string();
    let mut ids = Vec::new();
    for prompt in ["task-a", "task-b", "task-c"] {
        let (_, d) = post_json(
            &base,
            &format!("/api/nodes/{node_id}/tasks"),
            Some(serde_json::json!({ "prompt": prompt })),
        )
        .await;
        ids.push((
            d["task_id"].as_str().unwrap().to_string(),
            d["session_id"].as_str().unwrap().to_string(),
        ));
    }
    let (tid_a, sid_a) = (ids[0].0.clone(), ids[0].1.clone());
    let sid_b = ids[1].1.clone();
    let (tid_c, sid_c) = (ids[2].0.clone(), ids[2].1.clone());
    // Walk the state machine: A stays running, C reaches its terminal done.
    let now = chrono::Utc::now().timestamp_millis();
    store
        .update_node_task_status(&tid_a, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&tid_c, NodeTaskStatus::Running, None, now)
        .await
        .unwrap();
    store
        .update_node_task_status(&tid_c, NodeTaskStatus::Done, None, now)
        .await
        .unwrap();
    (srv, node_id, sid_a, sid_b, sid_c)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sweep_removes_terminal_and_skips_running() {
    let (srv, node_id, sid_a, sid_b, sid_c) = setup().await;
    let base = srv.base.clone();
    let store = srv.store.clone();

    // C carries a message row so the cascade is observable.
    store
        .append_message(&sid_c, &opencoder_core::Message::user("m1", "done work"))
        .await
        .unwrap();
    assert_eq!(store.load_messages(&sid_c).await.unwrap().len(), 1);

    let (status, v) = delete_json(&base, &format!("/api/nodes/{node_id}/dialogs")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["removed"], 1, "only the done dialog is removed");
    let skipped = v["skipped"].as_array().unwrap();
    let skipped_ids: Vec<&str> = skipped.iter().map(|s| s.as_str().unwrap()).collect();
    assert_eq!(skipped_ids.len(), 2, "running + pending survive");
    assert!(skipped.contains(&serde_json::Value::from(sid_a.as_str())));
    assert!(skipped.contains(&serde_json::Value::from(sid_b.as_str())));

    // Dialogs index now shows exactly the survivors.
    let (_, dv) = get_json(&base, &format!("/api/nodes/{node_id}/dialogs")).await;
    let ids: Vec<&str> = dv["dialogs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [sid_b.as_str(), sid_a.as_str()],
        "pending B is the newest dialog"
    );

    // Durable side: terminal session + its cascade are gone, survivors intact.
    assert!(store.get_session(&sid_c).await.unwrap().is_none());
    assert!(
        store.load_messages(&sid_c).await.unwrap().is_empty(),
        "messages cascade"
    );
    assert!(
        store
            .get_node_task_by_session(&sid_c)
            .await
            .unwrap()
            .is_none(),
        "task row cascades"
    );
    assert!(store.get_session(&sid_a).await.unwrap().is_some());
    assert!(store.get_session(&sid_b).await.unwrap().is_some());

    // A second sweep at the same task states is a no-op.
    let (_, v2) = delete_json(&base, &format!("/api/nodes/{node_id}/dialogs")).await;
    assert_eq!(v2["removed"], 0);
    assert_eq!(v2["skipped"].as_array().unwrap().len(), 2);
    let (_, dv2) = get_json(&base, &format!("/api/nodes/{node_id}/dialogs")).await;
    assert_eq!(dv2["dialogs"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sweep_answers_404_for_unknown_node() {
    let srv = spawn_server().await;
    let (status, v) = delete_json(&srv.base, "/api/nodes/nope/dialogs").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(v["ok"], false);
}
