//! Brain surface: capability CRUD + search (mock embedder), target binding,
//! plan lifecycle with a scripted planner LLM, preview and dispatch
//! (unkeyed execution + keyed idempotent receipts).

use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, TOKEN};

const TOPIC_A: &str = "db migration plan";
const THRESH: f64 = 0.98;

fn capability_payload(summary: &str) -> serde_json::Value {
    json!({
        "capability_type": "tool-usage",
        "summary": summary,
        "input_desc": "a work request",
        "output_desc": "completed work",
        "eng_inputs": ["exemplar input"],
    })
}

async fn seed_cap(h: &Harness, summary: &str) -> String {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/capabilities",
            Some(capability_payload(summary)),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["capability"]["id"].as_str().unwrap().to_string()
}

/// Queues one planner LLM round-trip returning a decision tree that
/// routes TOPIC_A to `cap`.
fn queue_plan(h: &Harness, cap: &str) {
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"db work\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"db work\"}}}}}}"
    );
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta(reply.clone()),
        opencoder_llm::LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
}

mod capabilities;
mod default_execution;
mod dispatch;
mod planning;
