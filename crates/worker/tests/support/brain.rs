#![allow(dead_code)]
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient, RequestPurpose};
use serde_json::{json, Value};
use std::sync::Mutex;
pub fn plan() -> Value {
    json!({"id":"plan-integration","version":1,"created_at":1,"changelog":"v2 fixture","plan":{"schema_version":2,"title":"Review","objective":"verified report","inputs":{"document":{"description":"Named document","source":{"kind":"external"},"schema":{"type":"object"},"required":true}},"instances":[{"id":"one","description":"Review evidence","capability_id":"builtin-agent-act","action":{"kind":"agent","target":"act","prompt":"Review document","output_mode":"json"},"inputs":["document"],"outputs":["report"]}],"outputs":{"report":{"description":"Verified report"}},"entry":["one"],"routes":[{"id":"finish","description":"Deliver verified output","outputs":["report"],"targets":[],"exits":[{"id":"done","description":"Deliver report","deliverables":["report"],"require_completed":true,"require_verified":true}]}]}})
}
pub fn doc() -> Value {
    json!({"document":{"name":"Requirement","markdown":"# Review"}})
}
pub fn output(content: &str, passed: bool) -> Value {
    json!({"content":content,"completion":{"passed":true,"evidence":["delivered"]},"verification":{"passed":passed,"evidence":["test evidence"]}})
}
#[derive(Default)]
pub struct GraphClient {
    pub requests: Mutex<Vec<ChatRequest>>,
}
impl ChatStream for GraphClient {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(request.clone());
        let value = if request.purpose == RequestPurpose::Planning {
            let input: Value = serde_json::from_str(&request.messages.last().unwrap().text())?;
            if input.get("receipt").is_some() {
                let receipt = input["receipt"].clone();
                let exit = input["exits"]
                    .as_array()
                    .unwrap()
                    .first()
                    .map(|v| v["id"].clone());
                let selected = if exit.is_some() {
                    json!([])
                } else {
                    json!([input["candidates"][0]["instance"]])
                };
                json!({"receipt":receipt,"reason":"Fixture verifies connected output","selected":selected,"exit":exit})
            } else {
                plan()["plan"].clone()
            }
        } else {
            let text = request
                .messages
                .iter()
                .map(|m| m.text())
                .collect::<Vec<_>>()
                .join("\n");
            let tail = text
                .split("Named output descriptions: ")
                .nth(1)
                .ok_or_else(|| anyhow::anyhow!("missing output contract"))?;
            let descriptions: Value = serde_json::from_str(tail.lines().next().unwrap())?;
            Value::Object(
                descriptions
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|key| (key.clone(), output("node-owned answer", true)))
                    .collect(),
            )
        };
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.try_send(LlmEvent::Completed {
            text: value.to_string(),
            tool_calls: vec![],
            usage: None,
        })
        .unwrap();
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "mock"
    }
    fn embed(&self, texts: &[String], model: &str) -> anyhow::Result<Vec<Vec<f32>>> {
        MockChatClient::new().embed(texts, model)
    }
}
pub async fn wait_phase(
    node: &opencoder_worker::Worker,
    reference: &opencoder_core::fleet::ExecutionRef,
    phase: &str,
) -> Value {
    use opencoder_node::fleet::NodeService;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let snapshot = node
                .handle(opencoder_core::fleet::NodeOperation::Brain {
                    execution: reference.clone(),
                    action: "snapshot".into(),
                    input: json!({}),
                })
                .await
                .body;
            if snapshot["phase"] == phase {
                return snapshot;
            }
            assert_ne!(snapshot["phase"], "blocked", "{snapshot}");
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("brain phase timeout")
}
