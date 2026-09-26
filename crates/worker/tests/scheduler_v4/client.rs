#![allow(dead_code)]
//! Deterministic layered model stub.
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
    reflect_once: bool,
}

impl LayeredClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reflecting_once() -> Self {
        Self {
            reflect_once: true,
            ..Self::default()
        }
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
                None if self.reflect_once
                    && instruction["run"]["round"].as_u64() == Some(1)
                    && instruction["run"]["layer"].as_u64()
                        == Some(instruction["plan"]["layers"].as_array().unwrap().len() as u64) =>
                {
                    dispatch_to(&instruction, 1)?.to_string()
                }
                None if !failed(&instruction)
                    && instruction["run"]["layer"].as_u64().unwrap_or(0)
                        == instruction["plan"]["layers"].as_array().unwrap().len() as u64 =>
                {
                    complete(&instruction).to_string()
                }
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
    let layer = if failed(instruction) {
        1
    } else {
        instruction["run"]["layer"].as_u64().unwrap_or(0) + 1
    };
    dispatch_to(instruction, layer)
}

fn dispatch_to(instruction: &Value, layer: u64) -> anyhow::Result<Value> {
    let mut assignments = vec![];
    let returning = layer <= instruction["run"]["layer"].as_u64().unwrap_or(0);
    for node in instruction["plan"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["layer_id"] == instruction["plan"]["layers"][layer as usize - 1]["layer_id"])
    {
        let cap_id = &node["capability_id"];
        let cap = instruction["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["capability_id"] == *cap_id)
            .unwrap();
        let inputs: serde_json::Map<_, _> = cap["required_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| {
                (
                    name.as_str().unwrap().to_string(),
                    json!({"kind":"root","name":"repo"}),
                )
            })
            .collect();
        assignments.push(json!({"node_id":node["node_id"],"capability_id":cap_id,"inputs":inputs,"reason":"one bounded milestone capability"}));
    }
    Ok(json!({
        "decision": "dispatch_layer",
        "layer": layer,
        "assessments": assessments(instruction),
        "reflection":if returning {Some("repair coding with the latest milestone evidence")} else {None},
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
        "assessments": assessments(instruction),
        "reason": "every layer succeeded",
        "evidence_execution_ids": evidence,
        "summary": "layered run complete",
    })
}

fn assessments(instruction: &Value) -> Value {
    let Some(layer) = instruction["run"]["layer"]
        .as_u64()
        .filter(|layer| *layer > 0)
    else {
        return json!({});
    };
    let id = instruction["plan"]["layers"][layer as usize - 1]["layer_id"]
        .as_str()
        .unwrap();
    let met = !instruction["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["activation"] == instruction["run"]["activation"] && op["status"] != "done");
    json!({id:{"met":met,"reason":"verified execution outputs"}})
}

fn failed(instruction: &Value) -> bool {
    instruction["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| {
            op["activation"] == instruction["run"]["activation"]
                && matches!(op["status"].as_str(), Some("error" | "cancelled"))
        })
}
