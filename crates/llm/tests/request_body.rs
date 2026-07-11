//! Contract test for `ChatRequest::to_body` reasoning_effort serialization.

use opencode_llm::ChatRequest;

fn req(reasoning: Option<&str>) -> ChatRequest {
    ChatRequest {
        model: "m".to_string(),
        messages: vec![],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: reasoning.map(|s| s.to_string()),
    }
}

#[test]
fn body_omits_reasoning_effort_when_none() {
    let body = req(None).to_body();
    assert!(
        body.get("reasoning_effort").is_none(),
        "field must be absent when None"
    );
}

#[test]
fn body_includes_reasoning_effort_when_set() {
    let body = req(Some("high")).to_body();
    assert_eq!(body["reasoning_effort"], serde_json::json!("high"));
}

#[test]
fn body_omits_reasoning_effort_when_empty() {
    let body = req(Some("   ")).to_body();
    assert!(
        body.get("reasoning_effort").is_none(),
        "blank value must be treated as unset"
    );
}
