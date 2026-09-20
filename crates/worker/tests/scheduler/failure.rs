use super::support::Fleet;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::{mpsc, Notify};

#[derive(Default)]
struct FailureClient {
    decisions: AtomicUsize,
    sibling_started: Arc<Notify>,
    held: Mutex<Vec<mpsc::Sender<LlmEvent>>>,
}

impl ChatStream for FailureClient {
    fn chat_stream(&self, request: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel(2);
        if request.purpose == RequestPurpose::Planning {
            self.decisions.fetch_add(1, Ordering::SeqCst);
            let decision = json!({"decision":"dispatch","capabilities":[
                {"capability_id":"builtin-agent-act","inputs":{"request":{"kind":"root","name":"failure"}}},
                {"capability_id":"builtin-operator","inputs":{"request":{"kind":"root","name":"sibling"}}}
            ],"reason":"test failure cancels active sibling","evidence_execution_ids":[]});
            tx.try_send(LlmEvent::Completed {
                text: decision.to_string(),
                tool_calls: vec![],
                usage: None,
            })?;
        } else if request.purpose != RequestPurpose::Conversation {
            tx.try_send(LlmEvent::Completed {
                text: "failure propagation acceptance".into(),
                tool_calls: vec![],
                usage: None,
            })?;
        } else if request
            .messages
            .last()
            .unwrap()
            .text()
            .contains("hold-sibling-marker")
        {
            self.held.lock().unwrap().push(tx);
            self.sibling_started.notify_one();
        } else {
            let started = self.sibling_started.clone();
            tokio::spawn(async move {
                started.notified().await;
                let _ = tx
                    .send(LlmEvent::Error("intentional child failure".into()))
                    .await;
            });
        }
        Ok(rx)
    }
}

#[tokio::test]
async fn child_failure_cancels_inflight_sibling_without_another_model_decision() {
    let client = Arc::new(FailureClient::default());
    let fleet = Fleet::new(1, client.clone()).await;
    let id = "brain-failure-cancellation";
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({
                "schema_version":3,"id":id,"objective":"Exercise failure propagation",
                "inputs":{"failure":"fail-child-marker","sibling":"hold-sibling-marker"},
                "capability_ids":["builtin-agent-act","builtin-operator"],"max_rounds":2
            }),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let reply = fleet
                .call("GET", &format!("/api/brain/runs/{id}"), Value::Null)
                .await;
            assert_eq!(reply.status, 200, "{reply:?}");
            let value = reply.body;
            if value["run"]["phase"] == "failed"
                && value["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|op| matches!(op["status"].as_str(), Some("error" | "cancelled")))
            {
                break value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("failure and sibling cancellation must converge");
    assert_eq!(snapshot["run"]["round"], 1);
    assert_eq!(client.decisions.load(Ordering::SeqCst), 1);
    let operations = snapshot["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2);
    let sibling = operations
        .iter()
        .find(|op| op["execution_kind"] == "operator")
        .unwrap();
    assert_eq!(sibling["status"], "cancelled");
    assert_eq!(sibling["cancel_requested"], true);
    let events = fleet
        .call(
            "GET",
            &format!("/api/brain/runs/{id}/events-page"),
            Value::Null,
        )
        .await;
    let events = events.body["events"].as_array().unwrap();
    assert!(events
        .iter()
        .any(|event| event["event_type"] == "run_failed"));
    assert!(!events
        .iter()
        .any(|event| event["event_type"] == "round_barrier_reached"));
    let detail = fleet
        .call(
            "GET",
            &format!(
                "/api/executions/{}",
                sibling["execution_id"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(detail.body["execution"]["status"], "cancelled");
    fleet.shutdown().await;
}
