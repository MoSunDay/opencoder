//! Contract test for `ChatRequest::to_body` reasoning_effort serialization.

use opencoder_llm::ChatRequest;

fn req(reasoning: Option<&str>) -> ChatRequest {
    ChatRequest {
        model: "m".to_string(),
        messages: vec![],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: reasoning.map(|s| s.to_string()),
        cache_salt: None,
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

#[test]
fn body_includes_cache_salt_when_set() {
    let req = ChatRequest {
        model: "m".to_string(),
        messages: Vec::new(),
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: Some("act:sess-123".into()),
    };
    let body = req.to_body();
    assert_eq!(body["cache_salt"], serde_json::json!("act:sess-123"));
}

#[test]
fn body_omits_cache_salt_when_none() {
    let req = ChatRequest {
        model: "m".to_string(),
        messages: Vec::new(),
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    };
    let body = req.to_body();
    assert!(
        body.get("cache_salt").is_none(),
        "field must be absent when None"
    );
}

#[test]
fn body_omits_cache_salt_when_empty() {
    let req = ChatRequest {
        model: "m".to_string(),
        messages: Vec::new(),
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: Some("   ".into()),
    };
    let body = req.to_body();
    assert!(
        body.get("cache_salt").is_none(),
        "blank value must be treated as unset"
    );
}
