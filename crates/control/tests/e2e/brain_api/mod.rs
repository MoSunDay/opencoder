//! Brain surface: capability CRUD + search (mock embedder), target binding,
//! plan lifecycle with a scripted planner LLM, preview and dispatch
//! (unkeyed execution + keyed idempotent receipts).

use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, TOKEN};

const TOPIC_A: &str = "db migration plan";

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

mod agents;
mod capabilities;
mod planning;
mod playbooks;
