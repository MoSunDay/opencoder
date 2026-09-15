#[path = "support/mod.rs"]
mod support;
use base64::Engine;
use opencoder_llm::{LlmEvent, MockChatClient};
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

fn answer(client: &MockChatClient, value: Value) {
    let text = value.to_string();
    client.queue_script(vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]);
}
fn dispatch(client: &MockChatClient, id: &str, mode: &str) {
    answer(
        client,
        json!({"operation":"dispatch","todos":[{"todo_id":id,"context_mode":mode}],"reason":"ready"}),
    );
}
fn step(client: &MockChatClient, id: &str, mode: &str, result: &str) {
    dispatch(client, id, mode);
    answer(
        client,
        json!({"status":"candidate","summary":format!("{id} summary"),"result":result,"verification":"checked","evidence_refs":["result.txt"],"recovery_context":{"summary":"checked","refs":[]}}),
    );
    answer(
        client,
        json!({"operation":"accept","reason":"evidence checked","mark_milestone":false}),
    );
}
fn spec() -> Value {
    json!({"schema_version":1,"id":"review-definition","name":"Review","objective":"review complete graph","constraints":["preserve files"],"todos":[
        {"id":"a","title":"A","requirement_background":"background","instructions":"execute A","acceptance":{"criteria":"tested"}},
        {"id":"b","title":"B","requirement_background":"background","instructions":"execute B","depends_on":["a"],"acceptance":{"criteria":"tested"}},
        {"id":"c","title":"C","requirement_background":"background","instructions":"execute C","depends_on":["b"],"acceptance":{"criteria":"tested"}},
        {"id":"other","title":"Independent","requirement_background":"background","instructions":"execute other","acceptance":{"criteria":"tested"}}
    ]})
}
async fn review(fleet: &Fleet, id: &str, query: &str) -> Value {
    let path = format!("/api/todo/workflows/{id}/review?{query}");
    let mut reply = fleet.call("GET", &path, Value::Null).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    if reply.body["encoding"] != "json-base64" {
        return reply.body;
    }
    let etag = reply.body["etag"].as_str().unwrap().to_owned();
    let mut bytes = vec![];
    loop {
        assert_eq!(reply.body["etag"], etag);
        assert_eq!(reply.body["offset"], bytes.len());
        bytes.extend(
            base64::engine::general_purpose::STANDARD
                .decode(reply.body["bytes_b64"].as_str().unwrap())
                .unwrap(),
        );
        if reply.body["eof"] == true {
            break;
        }
        reply = fleet
            .call(
                "GET",
                &format!("{path}&offset={}&etag={etag}", bytes.len()),
                Value::Null,
            )
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
    }
    serde_json::from_slice(&bytes).unwrap()
}
async fn submit(fleet: &Fleet, id: &str) {
    let response = fleet
        .call(
            "POST",
            "/api/executions",
            json!({"id":id,"kind":"todos","input":{"spec":spec()}}),
        )
        .await;
    assert_eq!(response.status, 202, "{response:?}");
}
async fn receipt_queued(fleet: &Fleet, id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let snapshot = review(fleet, id, "section=overview").await;
            if snapshot["controls"][0]["phase"] == "queued" {
                break;
            }
            assert_ne!(snapshot["controls"][0]["phase"], "failed", "{snapshot}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn workflow_review_rerun_and_large_context_cross_the_fleet_boundary() {
    let _config = isolated_config();
    let client = Arc::new(MockChatClient::new());
    let fleet = Fleet::new(1, client.clone()).await;
    let preview = fleet
        .call(
            "POST",
            "/api/todo/context-preview",
            json!({"spec":spec(),"todo_id":"b"}),
        )
        .await;
    assert_eq!(preview.status, 200);
    assert_eq!(preview.body["workflow_objective"], "review complete graph");
    assert_eq!(preview.body["accepted_dependencies"][0]["todo_id"], "a");
    assert_eq!(
        fleet
            .call(
                "POST",
                "/api/todo/context-preview",
                json!({"spec":spec(),"todo_id":"missing"})
            )
            .await
            .status,
        404
    );
    let large = "完整依赖结果".repeat(6_000);
    for id in ["a", "b", "c", "other"] {
        step(
            &client,
            id,
            "new",
            if id == "a" { &large } else { "result" },
        );
    }
    answer(
        &client,
        json!({"operation":"complete","reason":"all accepted"}),
    );
    let id = "todos-review-fleet";
    submit(&fleet, id).await;
    let done = settled(&fleet.nodes[0], id).await;
    assert_eq!(
        done["execution"]["status"], "done",
        "error={} result={}",
        done["error"], done["result"]
    );
    let overview = review(&fleet, id, "section=overview").await;
    assert_eq!(overview["nodes"].as_array().unwrap().len(), 4);
    assert_eq!(overview["workflow"]["status"], "completed");
    let original = review(&fleet, id, "section=node&todo_id=b").await;
    assert_eq!(original["preview"]["affected"], json!(["b", "c"]));
    assert_eq!(
        original["state"]["session_history"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let a = review(&fleet, id, "section=node&todo_id=a").await;
    assert_eq!(a["state"]["candidate"]["result"], large);
    let history = review(&fleet, id, "section=history").await;
    assert!(history["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "todos_dispatched"));
    let parent = overview["workflow"]["parent_session_id"].as_str().unwrap();
    let messages = review(&fleet, id, &format!("section=messages&session_id={parent}")).await;
    assert!(!messages["chunks"].as_array().unwrap().is_empty());
    assert_eq!(
        fleet
            .call(
                "GET",
                &format!("/api/todo/workflows/{id}/review?section=messages&session_id=foreign"),
                Value::Null
            )
            .await
            .status,
        403
    );
    let path = format!("/api/todo/workflows/{id}/rerun");
    let request = json!({"request_id":"rerun-review-b","todo_id":"b","reason":"review correction","expected_generation":overview["workflow"]["generation"]});
    let mut stale = request.clone();
    stale["expected_generation"] = json!(0);
    assert_eq!(fleet.call("POST", &path, stale).await.status, 409);
    step(&client, "b", "fork", "revised B");
    step(&client, "c", "fork", "revised C");
    answer(
        &client,
        json!({"operation":"complete","reason":"rerun accepted"}),
    );
    assert_eq!(fleet.call("POST", &path, request.clone()).await.status, 202);
    assert_eq!(fleet.call("POST", &path, request.clone()).await.status, 200);
    receipt_queued(&fleet, id).await;
    let done = settled(&fleet.nodes[0], id).await;
    assert_eq!(
        done["execution"]["status"], "done",
        "error={} result={}",
        done["error"], done["result"]
    );
    let next = review(&fleet, id, "section=overview").await;
    assert_eq!(
        next["workflow"]["world_epoch"].as_u64().unwrap(),
        overview["workflow"]["world_epoch"].as_u64().unwrap() + 1
    );
    let b = review(&fleet, id, "section=node&todo_id=b").await;
    assert_eq!(b["state"]["session_history"].as_array().unwrap().len(), 2);
    assert_ne!(
        b["state"]["active_session_id"],
        original["state"]["active_session_id"]
    );
    assert_eq!(b["state"]["candidate"]["result"], "revised B");
    assert_eq!(
        review(&fleet, id, "section=node&todo_id=a").await["state"],
        a["state"]
    );
    assert_eq!(fleet.call("POST", &path, request.clone()).await.status, 200);
    let mut conflicting = request;
    conflicting["reason"] = json!("different");
    assert_eq!(fleet.call("POST", &path, conflicting).await.status, 409);
    fleet.shutdown().await;
}

#[tokio::test]
async fn rerun_interrupts_an_active_child_before_forking() {
    let _config = isolated_config();
    let client = Arc::new(MockChatClient::new());
    let fleet = Fleet::new(1, client.clone()).await;
    dispatch(&client, "a", "new");
    client.queue_hang(Arc::new(tokio::sync::Notify::new()));
    let id = "todos-review-active";
    submit(&fleet, id).await;
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = review(&fleet, id, "section=overview").await;
            if s["nodes"][0]["status"] == "running" && client.call_count() >= 2 {
                break s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let path = format!("/api/todo/workflows/{id}/rerun");
    let mut request = json!({"request_id":"rerun-active","todo_id":"b","reason":"retry","expected_generation":snapshot["workflow"]["generation"]});
    assert_eq!(fleet.call("POST", &path, request.clone()).await.status, 409);
    request["todo_id"] = json!("a");
    for todo in ["a", "b", "c", "other"] {
        step(
            &client,
            todo,
            if todo == "a" { "fork" } else { "new" },
            "result",
        );
    }
    answer(
        &client,
        json!({"operation":"complete","reason":"all accepted"}),
    );
    let accepted = fleet.call("POST", &path, request).await;
    assert_eq!(accepted.status, 202, "{accepted:?}");
    receipt_queued(&fleet, id).await;
    let done = settled(&fleet.nodes[0], id).await;
    assert_eq!(
        done["execution"]["status"], "done",
        "error={} result={}",
        done["error"], done["result"]
    );
    let node = review(&fleet, id, "section=node&todo_id=a").await;
    assert_eq!(
        node["state"]["session_history"].as_array().unwrap().len(),
        2
    );
    fleet.shutdown().await;
}
