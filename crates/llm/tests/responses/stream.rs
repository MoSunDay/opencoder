use super::support::*;
use opencoder_core::ProviderConfig;
use opencoder_llm::LlmEvent;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn streams_unicode_reasoning_and_interleaved_calls_without_duplicate_text() {
    let mut message = answer("你好");
    message["phase"] = json!("commentary");
    let first = call("call_a", "read", json!({"path":"a"}));
    let second = call("call_b", "read", json!({"path":"b"}));
    let mut first_start = first.clone();
    first_start["arguments"] = json!("");
    first_start["status"] = json!("in_progress");
    let mut second_start = second.clone();
    second_start["arguments"] = json!("");
    second_start["status"] = json!("in_progress");
    let events = vec![
        json!({"type":"response.created","response":{"id":"resp_1"}}),
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"plan"}),
        json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"你好"}),
        json!({"type":"response.output_text.done","output_index":1,"content_index":0,"text":"你好"}),
        json!({"type":"response.output_item.added","output_index":2,"item":first_start}),
        json!({"type":"response.output_item.added","output_index":3,"item":second_start}),
        json!({"type":"response.function_call_arguments.delta","output_index":3,"item_id":"fc_call_b","delta":"{\"path\":"}),
        json!({"type":"response.function_call_arguments.delta","output_index":2,"item_id":"fc_call_a","delta":"{\"path\":\"a\"}"}),
        json!({"type":"response.function_call_arguments.delta","output_index":3,"item_id":"fc_call_b","delta":"\"b\"}"}),
        json!({"type":"response.output_item.done","output_index":0,"item":reasoning()}),
        json!({"type":"response.output_item.done","output_index":1,"item":message}),
        json!({"type":"response.output_item.done","output_index":2,"item":first}),
        completed(vec![reasoning(), message.clone(), first, second]),
    ];
    let mut reply = Reply::events(events);
    reply.chunk_size = 1;
    let server = serve(vec![reply]).await;
    let events = collect(&client(&config(&server.url)), request()).await;
    let text: String = events
        .iter()
        .filter_map(|e| {
            if let LlmEvent::TextDelta(s) = e {
                Some(s.as_str())
            } else {
                None
            }
        })
        .collect();
    let thinking: String = events
        .iter()
        .filter_map(|e| {
            if let LlmEvent::ReasoningDelta(s) = e {
                Some(s.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(text, "你好");
    assert_eq!(thinking, "plan");
    assert!(
        matches!(events.last(),Some(LlmEvent::Completed{text,tool_calls,usage:Some(u)})
        if text=="你好" && tool_calls.len()==2 && tool_calls[0].id=="call_a" && tool_calls[1].input["path"]=="b"
        && u.reasoning_tokens==20 && u.cache_read_tokens==80 && u.cache_creation_tokens==10 && u.total_tokens==130)
    );
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
    assert_eq!(state.output[1]["phase"], "commentary");
    assert_eq!(state.output[0]["encrypted_content"], "opaque-fixture");
    let requests = server.requests.lock().unwrap();
    assert!(requests[0].0.starts_with("POST /responses "));
    assert!(requests[0]
        .0
        .to_lowercase()
        .contains("authorization: bearer fixture-key"));
    assert_eq!(requests[0].1["model"], "gpt-6-astra");
}

#[tokio::test]
async fn json_response_and_refusal_complete_normally() {
    let refusal = json!({"type":"message","id":"msg_1","role":"assistant","content":[{"type":"refusal","refusal":"Cannot do that"}],"status":"completed"});
    let server = serve(vec![Reply::json(response(vec![refusal]))]).await;
    let events = collect(&client(&config(&server.url)), request()).await;
    assert!(matches!(events.last(),Some(LlmEvent::Completed{text,..})if text=="Cannot do that"));
}

#[tokio::test]
async fn terminal_failures_never_commit_tools_or_protocol_state() {
    let bad_call = json!({"type":"function_call","call_id":"c","name":"read","arguments":"{bad"});
    let cases = vec![
        json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"failed"}}}),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
        json!({"type":"error","message":"bad request"}),
        completed(vec![bad_call]),
        completed(vec![json!({"type":"unexpected_tool"})]),
        completed(vec![]),
    ];
    for terminal in cases {
        let server = serve(vec![Reply::events(vec![terminal])]).await;
        let events = collect(&client(&config(&server.url)), request()).await;
        assert!(
            matches!(events.last(), Some(LlmEvent::Error(_))),
            "{events:?}"
        );
        assert!(!events
            .iter()
            .any(|e| matches!(e, LlmEvent::Completed { .. } | LlmEvent::ProviderState(_))));
        assert_eq!(
            server.requests.lock().unwrap().len(),
            1,
            "terminal errors must not retry"
        );
    }
}

#[tokio::test]
async fn missing_terminal_retries_then_fails_and_regeneration_starts_fresh() {
    let partial = Reply::events(vec![
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"discard"}),
    ]);
    let server = serve(vec![
        partial.clone(),
        Reply::events(vec![completed(vec![answer("fresh")])]),
    ])
    .await;
    let events = collect(&client(&config(&server.url)), request()).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, LlmEvent::Retrying { .. })));
    assert!(matches!(events.last(),Some(LlmEvent::Completed{text,..})if text=="fresh"));
    let server = serve(vec![partial.clone(), partial.clone(), partial]).await;
    let events = collect(&client(&config(&server.url)), request()).await;
    assert!(matches!(events.last(), Some(LlmEvent::Error(_))));
    assert!(!events
        .iter()
        .any(|e| matches!(e, LlmEvent::Completed { .. })));
}

#[tokio::test]
async fn completed_event_ends_stream_without_waiting_for_connection_close() {
    let mut reply = Reply::events(vec![completed(vec![answer("done")])]);
    reply.hold = Duration::from_secs(5);
    let server = serve(vec![reply]).await;
    let events = tokio::time::timeout(
        Duration::from_secs(1),
        collect(&client(&config(&server.url)), request()),
    )
    .await
    .unwrap();
    assert!(matches!(events.last(), Some(LlmEvent::Completed { .. })));
}

#[tokio::test]
async fn mixed_provider_requests_choose_their_own_protocol() {
    let chat = json!({"choices":[{"delta":{"content":"chat"},"finish_reason":"stop"}]});
    let server = serve(vec![
        Reply::events(vec![completed(vec![answer("responses")])]),
        Reply::events(vec![chat]),
    ])
    .await;
    let mut cfg = config(&server.url);
    cfg.providers.insert(
        "small".into(),
        ProviderConfig {
            base_url: server.url.clone(),
            api_key: Some("small-key".into()),
            ..Default::default()
        },
    );
    let c = client(&cfg);
    assert!(
        matches!(collect(&c,request()).await.last(),Some(LlmEvent::Completed{text,..})if text=="responses")
    );
    let mut small = request();
    small.model = "small/tiny".into();
    assert!(
        matches!(collect(&c,small).await.last(),Some(LlmEvent::Completed{text,..})if text=="chat")
    );
    let captured = server.requests.lock().unwrap();
    assert!(captured[1].0.starts_with("POST /chat/completions "));
    assert!(captured[1].0.to_lowercase().contains("bearer small-key"));
    assert_eq!(captured[1].1["model"], "tiny");
    assert!(captured[1].1.get("input").is_none());
}

#[tokio::test]
async fn auth_errors_are_immediate_and_dropping_consumer_cancels() {
    for status in [401, 403] {
        let mut reply = Reply::json(json!({"error":{"message":"denied"}}));
        reply.status = status;
        let server = serve(vec![reply]).await;
        let events = collect(&client(&config(&server.url)), request()).await;
        assert!(matches!(events.last(),Some(LlmEvent::Error(e))if e.contains(&status.to_string())));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
    let mut reply = Reply::events(vec![
        json!({"type":"response.created","response":{"id":"resp_1"}}),
    ]);
    reply.hold = Duration::from_secs(10);
    let server = serve(vec![reply]).await;
    let rx = client(&config(&server.url)).chat_stream(request()).unwrap();
    drop(rx);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(server.requests.lock().unwrap().len() <= 1);
}
