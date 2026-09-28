//! Human input travels through Control, the durable Brain event, and the
//! running child session without crossing the current layer barrier.
#[path = "scheduler_v4/client.rs"]
mod client;
mod support;

use client::LayeredClient;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use support::Fleet;

struct HeldChild {
    brain: LayeredClient,
    held: AtomicBool,
    release: Arc<tokio::sync::Notify>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl HeldChild {
    fn new() -> Self {
        Self {
            brain: LayeredClient::new(),
            held: AtomicBool::new(false),
            release: Arc::new(tokio::sync::Notify::new()),
            requests: Mutex::new(vec![]),
        }
    }

    fn saw_guidance(&self) -> bool {
        self.requests.lock().unwrap().iter().any(|request| {
            request.messages.iter().any(|message| {
                message
                    .text()
                    .contains("Apply the new human verification constraint now")
            })
        })
    }
}

impl ChatStream for HeldChild {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(request.clone());
        if request.purpose == RequestPurpose::Conversation
            && !self.held.swap(true, Ordering::SeqCst)
        {
            let release = self.release.clone();
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            tokio::spawn(async move {
                release.notified().await;
                let _ = sender
                    .send(LlmEvent::Completed {
                        text: "node-owned child result".into(),
                        tool_calls: vec![],
                        usage: None,
                    })
                    .await;
            });
            Ok(receiver)
        } else {
            self.brain.chat_stream(request)
        }
    }
}

fn request(id: &str) -> Value {
    json!({"id":id,"schema_version":7,"inputs":{"repo":"opencoder"},"plan":{
        "schema_version":7,"title":"human guidance","objective":"inspect repository",
        "nodes":[{"node_id":"scan","layer_id":"scan-layer","title":"Scan",
            "objective":"scan repository","capability_id":"builtin-agent-act"}],
        "layers":[{"layer_id":"scan-layer","title":"Scan","task":"scan repository",
            "objective":"inspect repository","success_criteria":"result produced"}],
        "transitions":[],"edges":[],"max_rounds":4
    }})
}

#[tokio::test]
async fn human_input_wakes_brain_and_guides_the_running_agent_through_control() {
    let model = Arc::new(HeldChild::new());
    let fleet = Fleet::new(1, model.clone()).await;
    let id = "brain-human-guidance-e2e";
    let created = fleet.call("POST", "/api/brain/runs", request(id)).await;
    assert_eq!(created.status, 202, "{created:?}");

    let running = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view = fleet
                .call("GET", &format!("/api/brain/runs/{id}/layered"), Value::Null)
                .await;
            if view.status == 200
                && view.body["run"]["phase"] == "waiting"
                && view.body["operations"][0]["status"] == "running"
                && model.held.load(Ordering::SeqCst)
            {
                break view.body;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("first layer did not start");
    assert_eq!(running["run"]["layer"], 1);

    let input = fleet
        .call(
            "POST",
            &format!("/api/brain/runs/{id}/inputs"),
            json!({"text":"Verify the new constraint"}),
        )
        .await;
    assert_eq!(input.status, 200, "{input:?}");
    assert_eq!(input.body["delivery"], "brain_event");

    let guided = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view = fleet
                .call("GET", &format!("/api/brain/runs/{id}/layered"), Value::Null)
                .await;
            if view.status == 200
                && view.body["events"].as_array().is_some_and(|events| {
                    events
                        .iter()
                        .any(|event| event["event_type"] == "guidance_processed")
                })
                && model.saw_guidance()
            {
                break view.body;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("guidance was not delivered to the child session");
    assert_eq!(
        guided["run"]["layer"], 1,
        "guide crossed the layer barrier: {guided}"
    );
    assert!(guided["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "human_input"
            && event["user_input"] == "Verify the new constraint"));

    model.release.notify_one();
    fleet.shutdown().await;
}
