//! Real HTTP/WS admission with a deterministic external capacity authority.
use crate::support::Harness;
use axum::{routing::post, Json, Router};
use reqwest::Method;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[tokio::test]
async fn ordinary_submission_dispatches_while_resource_authority_is_unavailable() {
    let h = Harness::new().await;
    let (status, _) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"agent-managed-waiting","kind":"agent",
                "input":{"prompt":"fixture","_resource_request":{"units":1}}
            })),
        )
        .await;
    assert_eq!(status, 202);
    let (status, index) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(json!({
                "id":"agent-ordinary","kind":"agent","input":{"prompt":"fixture"}
            })),
        )
        .await;
    // Both queueing and Node acceptance return 202; the journal proves that
    // ordinary work actually reached the Node while managed work did not.
    assert_eq!(status, 202, "{index}");
    assert_eq!(h.node.journal_ids(), vec!["agent-ordinary"]);
    let (status, index) = h
        .req(
            Method::GET,
            "/api/executions/agent-managed-waiting/index",
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(index["status"], "pending");
}

#[tokio::test]
async fn resource_queue_waits_then_dispatches_in_order_without_duplicate_node_creates() {
    let h = Harness::new().await;
    let capacity = Arc::new(Mutex::new((0usize, Vec::<String>::new(), 0usize)));
    let app = Router::new().route(
        "/resources",
        post({
            let capacity = capacity.clone();
            move |Json(input): Json<Value>| {
                let capacity = capacity.clone();
                async move {
                    let mut state = capacity.lock().await;
                    let id = input["execution_id"].as_str().unwrap().to_owned();
                    state.2 += 1;
                    if state.1.contains(&id) {
                        return Json(json!({"status":"granted"}));
                    }
                    if state.0 == 0 {
                        return Json(json!({"status":"waiting"}));
                    }
                    state.0 -= 1;
                    state.1.push(id);
                    Json(json!({"status":"granted"}))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/resources", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let credentials = tempfile::tempdir().unwrap();
    let token = credentials.path().join("resource-provider.token");
    std::fs::write(&token, "isolated-resource-fixture").unwrap();
    let (status, _) = h
        .req(
            Method::PUT,
            "/api/resource-admission-provider",
            Some(json!({
                "endpoint":endpoint,"token_file":token
            })),
        )
        .await;
    assert_eq!(status, 200);
    for id in ["agent-z-first", "agent-a-second"] {
        let request = json!({"id":id,"kind":"agent","input":{"prompt":"fixture","_resource_request":{"units":1}}});
        for _ in 0..2 {
            let (status, index) = h
                .req(Method::POST, "/api/executions", Some(request.clone()))
                .await;
            assert_eq!(status, 202, "{index}");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        while capacity.lock().await.2 < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        h.node.journal_ids().is_empty(),
        "waiting jobs must never reach the Node"
    );
    capacity.lock().await.0 = 1;
    tokio::time::timeout(Duration::from_secs(20), async {
        while h.node.journal_ids().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.node.journal_ids(), vec!["agent-z-first"]);
    capacity.lock().await.0 = 1;
    tokio::time::timeout(Duration::from_secs(20), async {
        while h.node.journal_ids().len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        capacity.lock().await.1,
        vec!["agent-z-first", "agent-a-second"]
    );
    assert_eq!(h.node.journal_ids().len(), 2);
    provider.abort();
}
