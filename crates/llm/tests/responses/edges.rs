use super::support::*;
use opencoder_llm::LlmEvent;
use serde_json::json;

#[tokio::test]
async fn shared_client_trace_body_matches_the_actual_responses_request() {
    let server = serve(vec![Reply::json(response(vec![answer("ok")]))]).await;
    let client: std::sync::Arc<dyn opencoder_llm::ChatStream> =
        std::sync::Arc::new(client(&config(&server.url)));
    let req = request();
    let traced = client.request_body(&req).unwrap();
    let mut rx = client.chat_stream(req).unwrap();
    while rx.recv().await.is_some() {}
    assert_eq!(traced, server.requests.lock().unwrap()[0].1);
    assert!(traced.get("input").is_some());
    assert!(traced.get("messages").is_none());
}

#[tokio::test]
async fn rate_limit_and_server_errors_retry_same_responses_request() {
    let replies = [429, 503].map(|status| Reply {
        status,
        ..Reply::json(json!({"error":{"message":"retry"}}))
    });
    let server = serve(
        replies
            .into_iter()
            .chain([Reply::json(response(vec![answer("ok")]))])
            .collect(),
    )
    .await;
    let cfg = config(&server.url);
    let events = collect(&client(&cfg), request()).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, LlmEvent::Retrying { .. }))
            .count(),
        2
    );
    assert!(matches!(events.last(),Some(LlmEvent::Completed{text,..}) if text=="ok"));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|(h, b)| h.starts_with("POST /responses ") && b == &requests[0].1));
}

#[tokio::test]
async fn identical_sequenced_event_is_deduplicated_and_conflicts_are_errors() {
    let delta = json!({"type":"response.output_text.delta", "output_index":0, "content_index":0,
        "item_id":"msg_1", "sequence_number":1, "delta":"hello"});
    let server = serve(vec![Reply::events(vec![
        delta.clone(),
        delta.clone(),
        completed(vec![answer("hello")]),
    ])])
    .await;
    let events = collect(&client(&config(&server.url)), request()).await;
    let chunks: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let LlmEvent::TextDelta(t) = e {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(chunks, ["hello"]);
    for terminal in [
        completed(vec![answer("conflicting answer")]),
        json!({"type":"response.output_text.delta", "output_index":0, "content_index":0, "sequence_number":1,"delta":"conflict"}),
        completed(vec![]),
    ] {
        let server = serve(vec![Reply::events(vec![delta.clone(), terminal])]).await;
        let events = collect(&client(&config(&server.url)), request()).await;
        assert!(matches!(events.last(), Some(LlmEvent::Error(_))));
        assert!(!events
            .iter()
            .any(|e| matches!(e, LlmEvent::Completed { .. } | LlmEvent::ProviderState(_))));
    }
}

#[tokio::test]
async fn complete_snapshot_emits_full_tool_arguments_and_keeps_message_phase() {
    let mut commentary = answer("working");
    commentary["phase"] = json!("commentary");
    commentary["id"] = json!("msg_commentary");
    let server = serve(vec![Reply::json(response(vec![
        commentary.clone(),
        reasoning(),
        call("one", "mcp_server_search", json!({"query":"needle"})),
    ]))])
    .await;
    let events = collect(&client(&config(&server.url)), request()).await;
    assert!(events.iter().any(|e|matches!(e,LlmEvent::ToolCallDelta{index:2,arguments}if arguments=="{\"query\":\"needle\"}")));
    let state = events
        .iter()
        .find_map(|e| {
            if let LlmEvent::ProviderState(s) = e {
                Some(s)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(state.output[0], commentary);
    assert!(
        matches!(events.last(),Some(LlmEvent::Completed{tool_calls,..})if tool_calls[0].name=="mcp_server_search")
    );
    let server = serve(vec![Reply::json(response(vec![commentary]))]).await;
    assert!(matches!(
        collect(&client(&config(&server.url)), request())
            .await
            .last(),
        Some(LlmEvent::Error(_))
    ));
}

#[test]
fn opaque_ciphertext_does_not_inflate_context_estimates_but_reasoning_usage_counts() {
    let mut message = opencoder_core::Message::assistant("a");
    message
        .blocks
        .push(opencoder_core::ContentBlock::text("done"));
    let base = opencoder_llm::estimate_messages(std::slice::from_ref(&message));
    message.provider_state = Some(opencoder_core::ProviderState {
        provider: "fixture".into(),
        base_url: "fixture".into(),
        model: "fixture".into(),
        output: vec![json!({"type":"reasoning","encrypted_content":"x".repeat(100_000)})],
    });
    message.usage.reasoning_tokens = 42;
    assert_eq!(opencoder_llm::estimate_messages(&[message]), base + 42);
}
