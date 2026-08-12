use super::*;

/// Regression guard: the default read timeout must stay at 600 s (10 min).
#[test]
fn default_read_timeout_is_600s() {
    assert_eq!(DEFAULT_READ_TIMEOUT, Duration::from_secs(600));
}

// ---- parse_usage: base fields + cache-token normalization ----

fn usage_json(s: &str) -> Value {
    serde_json::from_str(s).expect("valid usage json")
}

#[test]
fn parse_usage_reads_openai_base_fields() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":100,"completion_tokens":40,"total_tokens":140}"#,
    ));
    assert_eq!(u.input_tokens, 100);
    assert_eq!(u.output_tokens, 40);
    assert_eq!(u.total_tokens, 140);
    assert_eq!(u.cache_read_tokens, 0);
    assert_eq!(u.cache_creation_tokens, 0);
}

#[test]
fn parse_usage_reads_anthropic_cache_fields() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,
               "cache_read_input_tokens":42,"cache_creation_input_tokens":7}"#,
    ));
    assert_eq!(u.cache_read_tokens, 42);
    assert_eq!(u.cache_creation_tokens, 7);
}

#[test]
fn parse_usage_reads_short_aliases() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,
               "cache_read":8,"cache_write":3}"#,
    ));
    assert_eq!(u.cache_read_tokens, 8);
    assert_eq!(u.cache_creation_tokens, 3);
}

#[test]
fn parse_usage_reads_openai_cached_tokens() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":300,"completion_tokens":20,"total_tokens":320,
               "prompt_tokens_details":{"cached_tokens":9000}}"#,
    ));
    assert_eq!(u.cache_read_tokens, 9000);
    assert_eq!(u.cache_creation_tokens, 0);
}

#[test]
fn parse_usage_prefers_explicit_anthropic_key_over_alias() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,
               "cache_read_input_tokens":7,"cache_read":9}"#,
    ));
    assert_eq!(u.cache_read_tokens, 7);
}

#[test]
fn parse_usage_missing_cache_fields_default_to_zero() {
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#,
    ));
    assert_eq!(u.cache_read_tokens, 0);
    assert_eq!(u.cache_creation_tokens, 0);
}

#[test]
fn parse_usage_empty_object_is_all_zeros() {
    let u = parse_usage(&usage_json("{}"));
    assert_eq!(u.input_tokens, 0);
    assert_eq!(u.output_tokens, 0);
    assert_eq!(u.total_tokens, 0);
    assert_eq!(u.cache_read_tokens, 0);
    assert_eq!(u.cache_creation_tokens, 0);
}

#[test]
fn parse_usage_derives_total_when_omitted() {
    // Regression (Bug 11): when `total_tokens` is absent, the sum of
    // `prompt_tokens + completion_tokens` must be used instead of silently
    // reporting 0 (some providers omit total_tokens entirely).
    let u = parse_usage(&usage_json(r#"{"prompt_tokens":100,"completion_tokens":50}"#));
    assert_eq!(u.input_tokens, 100);
    assert_eq!(u.output_tokens, 50);
    assert_eq!(u.total_tokens, 150, "missing total must fall back to input+output");
}

#[test]
fn parse_usage_preserves_explicit_total() {
    // An explicit `total_tokens` is authoritative and must be preserved
    // even when it differs from input+output.
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":100,"completion_tokens":50,"total_tokens":999}"#,
    ));
    assert_eq!(u.total_tokens, 999);
}

#[test]
fn parse_usage_derives_total_when_explicit_zero() {
    // A present-but-zero `total_tokens` is treated as absent (some providers
    // send 0 as a placeholder) and falls back to input+output.
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":7,"completion_tokens":3,"total_tokens":0}"#,
    ));
    assert_eq!(u.total_tokens, 10, "explicit 0 total must fall back to input+output");
}

#[test]
fn parse_usage_total_saturates_on_overflow() {
    // Bug 4: input+output must saturate instead of wrapping on overflow.
    let u = parse_usage(&usage_json(
        r#"{"prompt_tokens":18446744073709551615,"completion_tokens":1}"#,
    ));
    assert_eq!(
        u.total_tokens, u64::MAX,
        "overflow on input+output must saturate, not wrap"
    );
}

#[test]
fn first_u64_returns_first_present_key() {
    let obj = usage_json(r#"{"a":10,"b":20}"#);
    assert_eq!(first_u64(&obj, &["a", "b"]), Some(10));
    assert_eq!(first_u64(&obj, &["b", "a"]), Some(20));
    assert_eq!(first_u64(&obj, &["missing"]), None);
}

// ---- extract_reasoning: provider-specific reasoning shapes ----

fn obj(s: &str) -> Value {
    serde_json::from_str(s).expect("valid json")
}

#[test]
fn extract_reasoning_reads_reasoning_content_string() {
    let v = obj(r#"{"reasoning_content":" think hard"}"#);
    assert_eq!(extract_reasoning(&v), Some(" think hard".to_string()));
}

#[test]
fn extract_reasoning_reads_alias_keys() {
    for key in [
        "reasoning",
        "thinking",
        "reasoning_summary",
        "chain_of_thought",
    ] {
        let v = obj(&format!(r#"{{"{key}":" step"}}"#));
        assert_eq!(
            extract_reasoning(&v),
            Some(" step".to_string()),
            "key {key}"
        );
    }
}

#[test]
fn extract_reasoning_joins_string_array() {
    let v = obj(r#"{"reasoning":["a","b","c"]}"#);
    assert_eq!(extract_reasoning(&v), Some("abc".to_string()));
}

#[test]
fn extract_reasoning_reads_structured_thinking_blocks() {
    let v = obj(r#"{"content":[{"type":"text","text":"hello"},
                           {"type":"thinking","text":" deep"},
                           {"type":"reasoning","content":" more"}]}"#);
    assert_eq!(extract_reasoning(&v), Some(" deep more".to_string()));
}

#[test]
fn extract_reasoning_prefers_explicit_key_over_structured() {
    let v =
        obj(r#"{"reasoning_content":"explicit","content":[{"type":"thinking","text":"other"}]}"#);
    assert_eq!(extract_reasoning(&v), Some("explicit".to_string()));
}

#[test]
fn extract_reasoning_ignores_plain_text_content() {
    let v = obj(r#"{"content":[{"type":"text","text":"just text"}]}"#);
    assert_eq!(extract_reasoning(&v), None);
}

#[test]
fn extract_reasoning_returns_none_when_absent() {
    let v = obj(r#"{"content":"plain"}"#);
    assert_eq!(extract_reasoning(&v), None);
}

#[test]
fn extract_reasoning_skips_empty_alias() {
    let v = obj(r#"{"reasoning_content":"","thinking":"real"}"#);
    assert_eq!(extract_reasoning(&v), Some("real".to_string()));
}

// ---- handle_event: emit_delta + finish_reason fallback emit ReasoningDelta ----

#[tokio::test]
async fn emit_delta_emits_reasoning_for_alias_field() {
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut text = String::new();
    let delta = obj(r#"{"reasoning":"what if"}"#);
    emit_delta(&delta, &mut tools, &mut text, &tx)
        .await
        .unwrap();
    drop(tx);
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::ReasoningDelta(ref s) if s == "what if"));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn emit_delta_emits_reasoning_for_structured_thinking() {
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut text = String::new();
    let delta = obj(r#"{"content":[{"type":"thinking","text":"deep"},
                            {"type":"text","text":"answer"}]}"#);
    emit_delta(&delta, &mut tools, &mut text, &tx)
        .await
        .unwrap();
    drop(tx);
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::ReasoningDelta(ref s) if s == "deep"));
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::TextDelta(ref s) if s == "answer"));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn emit_delta_emits_flat_reasoning_before_flat_text() {
    // Some providers put the last reasoning token and first answer token in
    // one flat delta. The channels have semantic order even though JSON object
    // fields do not: reasoning must close before answer text starts.
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut text = String::new();
    let delta = obj(r#"{"content":"Now","reasoning_content":"."}"#);
    emit_delta(&delta, &mut tools, &mut text, &tx)
        .await
        .unwrap();
    drop(tx);

    let first = rx.recv().await.unwrap();
    assert!(matches!(first, LlmEvent::ReasoningDelta(ref s) if s == "."));
    let second = rx.recv().await.unwrap();
    assert!(matches!(second, LlmEvent::TextDelta(ref s) if s == "Now"));
    assert_eq!(text, "Now");
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn handle_event_emits_reasoning_from_message_fallback() {
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut usage = None;
    let mut finished = false;
    let mut text = String::new();
    let mut streamed_reasoning = false;
    let parsed = obj(r#"{"choices":[{"finish_reason":"stop","delta":{},
                             "message":{"reasoning_content":"final think","content":"done"}}]}"#);
    handle_event(
        &parsed,
        &mut tools,
        &mut usage,
        &mut finished,
        &mut text,
        &mut streamed_reasoning,
        &tx,
    )
    .await
    .unwrap();
    assert!(finished);
    assert!(streamed_reasoning);
    drop(tx);
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::ReasoningDelta(ref s) if s == "final think"));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn handle_event_skips_message_fallback_when_no_reasoning() {
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut usage = None;
    let mut finished = false;
    let mut text = String::new();
    let mut streamed_reasoning = false;
    let parsed = obj(r#"{"choices":[{"finish_reason":"stop","delta":{},
                             "message":{"content":"done"}}]}"#);
    handle_event(
        &parsed,
        &mut tools,
        &mut usage,
        &mut finished,
        &mut text,
        &mut streamed_reasoning,
        &tx,
    )
    .await
    .unwrap();
    assert!(!streamed_reasoning);
    drop(tx);
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn message_fallback_does_not_double_emit_after_streamed_reasoning() {
    // Provider streams reasoning via `delta.reasoning_content` across frames,
    // then repeats it wholesale in the final `choice.message`. The cross-frame
    // guard must suppress the fallback so the UI Thinking block is not
    // duplicated (and tool-turn reasoning is not double-persisted).
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut usage = None;
    let mut finished = false;
    let mut text = String::new();
    let mut streamed_reasoning = false;

    // Frame 1: reasoning streamed as delta.
    let frame1 = obj(r#"{"choices":[{"delta":{"reasoning_content":"think step one"}}]}"#);
    handle_event(
        &frame1,
        &mut tools,
        &mut usage,
        &mut finished,
        &mut text,
        &mut streamed_reasoning,
        &tx,
    )
    .await
    .unwrap();
    assert!(streamed_reasoning);

    // Frame 2 (last): same reasoning repeated in `choice.message`.
    let frame2 = obj(r#"{"choices":[{"finish_reason":"stop","delta":{},
                             "message":{"reasoning_content":"think step one","content":"done"}}]}"#);
    handle_event(
        &frame2,
        &mut tools,
        &mut usage,
        &mut finished,
        &mut text,
        &mut streamed_reasoning,
        &tx,
    )
    .await
    .unwrap();
    assert!(finished);

    drop(tx);
    // Exactly one ReasoningDelta from the whole turn — no duplicate.
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::ReasoningDelta(ref s) if s == "think step one"));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn message_fallback_fires_when_no_delta_reasoning_streamed() {
    // Provider delivers reasoning ONLY via `choice.message` (no delta
    // reasoning anywhere). The fallback must still fire — the guard only
    // suppresses duplicates, not legitimate single-channel providers.
    let (tx, mut rx) = mpsc::channel::<LlmEvent>(16);
    let mut tools = ToolAccumulator::default();
    let mut usage = None;
    let mut finished = false;
    let mut text = String::new();
    let mut streamed_reasoning = false;

    let parsed = obj(r#"{"choices":[{"finish_reason":"stop",
                             "delta":{"content":"answer text"},
                             "message":{"reasoning_content":"only message think"}}]}"#);
    handle_event(
        &parsed,
        &mut tools,
        &mut usage,
        &mut finished,
        &mut text,
        &mut streamed_reasoning,
        &tx,
    )
    .await
    .unwrap();
    assert!(streamed_reasoning);

    drop(tx);
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::TextDelta(ref s) if s == "answer text"));
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, LlmEvent::ReasoningDelta(ref s) if s == "only message think"));
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn stream_task_exits_promptly_after_rx_drop() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let _ = sock.read(&mut buf).await;
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
            .await
            .unwrap();
        sock.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n")
            .await
            .unwrap();
        let _ = sock.read(&mut buf).await;
        let _ = closed_tx.send(());
    });

    let client = ChatClient::new(
        &format!("http://127.0.0.1:{port}"),
        "test-key",
        &[],
        None,
    )
    .unwrap();

    let req = ChatRequest {
        model: "test-model".to_string(),
        messages: vec![],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    };

    let mut rx = client.chat_stream(req).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(rx);

    assert!(
        tokio::time::timeout(Duration::from_secs(5), closed_rx)
            .await
            .is_ok(),
        "stream task did not close the connection within 5s after rx drop"
    );
}
