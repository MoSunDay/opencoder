mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::{self, NodeService};
use opencoder_store::SessionFilter;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

#[tokio::test]
async fn server_routes_by_id_and_disconnect_does_not_stop_accepted_work() {
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let node = worker(dir.path(), client.clone()).await;
    let state = opencoder_control::new_state(
        dir.path().join("work"),
        dir.path().join("server"),
        Some(client.clone()),
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = format!("http://{}", listener.local_addr().unwrap());
    let app = opencoder_control::build_app(state.clone(), None, false);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = remote.clone();
    let service: Arc<dyn NodeService> = Arc::new(node.clone());
    let channel = tokio::spawn(async move { fleet::run(&url, "test-token", service).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !state.hub.views().await.iter().any(|n| n.online) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let hang = Arc::new(tokio::sync::Notify::new());
    client.queue_hang(hang.clone());
    let request = CreateExecution {
        id: "agent-channel".into(),
        kind: ExecutionKind::Agent,
        target: None,
        input: json!({"prompt":"secret stays local"}),
        node_id: None,
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let accepted = http
        .post(format!("{remote}/api/executions"))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 202, "{}", accepted.text().await.unwrap());
    assert!(state
        .store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()
        .is_empty());
    let index = state.fleet.index(&request.id).await.unwrap().unwrap();
    assert_eq!(index.node_id, node.registration().id);
    assert_eq!(index.kind, ExecutionKind::Agent);
    let index_json = serde_json::to_value(&index).unwrap();
    assert_eq!(index_json.as_object().unwrap().len(), 5);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while client.call_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(node.snapshot().active_agent_loops > 0);
    channel.abort();
    let _ = channel.await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.hub.views().await.iter().any(|n| n.online) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        http.get(format!("{remote}/api/executions/{}", request.id))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        state
            .fleet
            .index(&request.id)
            .await
            .unwrap()
            .unwrap()
            .node_id,
        index.node_id
    );
    hang.notify_one();
    let _ = settled(&node, &request.id).await;
    let url = remote.clone();
    let service: Arc<dyn NodeService> = Arc::new(node.clone());
    let reconnect = tokio::spawn(async move { fleet::run(&url, "test-token", service).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !state.hub.views().await.iter().any(|n| n.online) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let calls = client.call_count();
    assert_eq!(
        http.post(format!("{remote}/api/executions"))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    assert_eq!(client.call_count(), calls);
    let detail: Value = http
        .get(format!("{remote}/api/executions/{}", request.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(support::messages::message_page_text(&detail).contains("secret stays local"));
    let events = http
        .get(format!("{remote}/api/executions/{}/events", request.id))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(events.contains("id:"));
    let raw = std::fs::read(dir.path().join("server/control.db")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("secret stays local"));
    reconnect.abort();
    server.abort();
    node.shutdown().await.unwrap();
}
