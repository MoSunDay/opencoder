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
