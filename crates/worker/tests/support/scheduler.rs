#![allow(dead_code)]
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Default)]
pub struct SchedulerClient {
    pub requests: Mutex<Vec<ChatRequest>>,
}
impl ChatStream for SchedulerClient {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(request.clone());
        let text = if request.purpose == RequestPurpose::Planning {
            let context: Value = serde_json::from_str(&request.messages.last().unwrap().text())?;
            if context["round"] == 0 {
                json!({"decision":"dispatch","capabilities":[{
                    "capability_id":"builtin-agent-act",
                    "inputs":{"request":{"kind":"root","name":"repo"}}
                }],"reason":"inspect repository","evidence_execution_ids":[]})
                .to_string()
            } else {
                json!({"decision":"complete","reason":"child supplied evidence",
                    "evidence_execution_ids":[context["operations"][0]["execution_id"]]})
                .to_string()
            }
        } else {
            "child scheduler output".into()
        };
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        })?;
        Ok(rx)
    }
}
pub fn request(id: &str) -> Value {
    json!({"schema_version":3,"id":id,"objective":"inspect repository",
        "inputs":{"repo":"opencoder"},"max_rounds":2})
}
pub async fn wait_phase(fleet: &super::Fleet, id: &str, phase: &str) -> Value {
    wait_phase_within(fleet, id, phase, 30).await
}

/// Terminal notices travel through an acknowledgement-gated outbox that
/// replays every heartbeat. Under heavy test-fleet load a convergence window
/// longer than the default 30s budget is required.
pub async fn wait_phase_within(fleet: &super::Fleet, id: &str, phase: &str, seconds: u64) -> Value {
    let mut last = Value::Null;
    let waited = tokio::time::timeout(std::time::Duration::from_secs(seconds), async {
        loop {
            let reply = fleet
                .call("GET", &format!("/api/brain/runs/{id}"), Value::Null)
                .await;
            assert_eq!(reply.status, 200, "{reply:?}");
            last = reply.body;
            if last["run"]["phase"] == phase {
                return;
            }
            assert!(
                !matches!(
                    last["run"]["phase"].as_str(),
                    Some("failed" | "blocked" | "cancelled")
                ),
                "{last}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "scheduler did not reach {phase}: {last}; indexes={:?}",
        fleet.nodes[0].indexes().await
    );
    last
}
