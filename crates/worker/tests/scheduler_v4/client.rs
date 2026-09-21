#![allow(dead_code)]
//! Deterministic v4 model stub.
//!
//! Every decision is derived from the instruction the node handed the model, so
//! a test never has to script the exact layer sequence: a layer context
//! dispatches one bounded task per node, a closing context completes the run.
//! `with` forces the decisions of a test that needs a different contract.
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct LayeredClient {
    pub requests: Mutex<Vec<ChatRequest>>,
    forced: Mutex<VecDeque<Value>>,
}

impl LayeredClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Force the next decisions, in order, before the derived script resumes.
    pub fn with(decisions: impl IntoIterator<Item = Value>) -> Self {
        let client = Self::default();
        client.forced.lock().unwrap().extend(decisions);
        client
    }

    /// Planning-purpose calls: one per layer decision plus one closing call.
    pub fn decisions(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.purpose == RequestPurpose::Planning)
            .count()
    }

    pub fn instructions(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.messages.last().unwrap().text())
            .collect()
    }
}

impl ChatStream for LayeredClient {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(request.clone());
        let text = if request.purpose == RequestPurpose::Planning {
            let instruction: Value =
                serde_json::from_str(&request.messages.last().unwrap().text())?;
            let forced = self.forced.lock().unwrap().pop_front();
            match forced {
                Some(decision) => decision.to_string(),
                None if instruction["closing"] == true => complete(&instruction).to_string(),
                None => dispatch(&instruction)?.to_string(),
            }
        } else {
            // Any other purpose belongs to a child body, never to the canvas.
            "node-owned child result".into()
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

/// Bind every required capability input: layer one to the root input, a later
/// layer to its direct upstream execution result.
fn dispatch(instruction: &Value) -> anyhow::Result<Value> {
    let mut assignments = vec![];
    for node in instruction["nodes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("layer instruction has no nodes: {instruction}"))?
    {
        let execution = node["upstream"]
            .as_array()
            .and_then(|upstream| upstream.first())
            .and_then(|upstream| upstream["execution_id"].as_str())
            .map(str::to_string);
        let mut inputs = serde_json::Map::new();
        for name in node["capability"]["required_inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let name = name.as_str().unwrap_or_default().to_string();
            let binding = match &execution {
                Some(execution_id) => {
                    json!({"kind":"execution","execution_id":execution_id,"path":""})
                }
                None => json!({"kind":"root","name":"repo"}),
            };
            inputs.insert(name, binding);
        }
        assignments.push(json!({
            "node_id": node["node_id"],
            "inputs": inputs,
            "reason": "one bounded node task",
        }));
    }
    Ok(json!({
        "decision": "dispatch_layer",
        "layer": instruction["layer"],
        "assignments": assignments,
        "reason": "dispatch every node of the next layer",
        "evidence_execution_ids": [],
    }))
}

/// The closing decision cites the successful attempts as completion evidence.
fn complete(instruction: &Value) -> Value {
    let evidence: Vec<Value> = instruction["operations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|operation| operation["status"] == "done")
        .map(|operation| operation["execution_id"].clone())
        .collect();
    json!({
        "decision": "complete",
        "reason": "every layer succeeded",
        "evidence_execution_ids": evidence,
        "summary": "layered run complete",
    })
}
