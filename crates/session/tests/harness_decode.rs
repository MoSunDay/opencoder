use opencoder_session::harness::codex::decode::{decode, Decoder};
use opencoder_session::SessionEvent;
use serde_json::json;

#[test]
fn text_snapshots_emit_only_new_text_and_completion_once() {
    let mut state = Decoder::default();
    state = decode(state, r#"{"type":"turn.started"}"#).unwrap().0;
    let mut events = vec![];
    let mut messages = vec![];
    for (event, text) in [
        ("item.started", "你"),
        ("item.updated", "你好"),
        ("item.completed", "你好！"),
        ("item.completed", "你好！"),
    ] {
        let result = decode(
            state,
            &json!({"type":event,"item":{"id":"a","type":"agent_message","text":text}}).to_string(),
        )
        .unwrap();
        state = result.0;
        events.extend(result.1.events);
        messages.extend(result.1.messages);
    }
    assert_eq!(
        events
            .iter()
            .filter_map(|e| if let SessionEvent::TextDelta(t) = e {
                Some(t.as_str())
            } else {
                None
            })
            .collect::<String>(),
        "你好！"
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text(), "你好！");
}

#[test]
fn all_tool_items_project_to_existing_tool_events() {
    for item in [
        json!({"type":"command_execution","command":"false","aggregated_output":"failed","exit_code":1,"status":"failed"}),
        json!({"type":"file_change","changes":[{"path":"a.rs","kind":"update"}],"status":"completed"}),
        json!({"type":"mcp_tool_call","server":"srv","tool":"read","arguments":{"x":1},"result":{"content":[]},"status":"completed"}),
        json!({"type":"web_search","query":"query","action":{"type":"search"}}),
        json!({"type":"collab_tool_call","tool":"spawn_agent","receiver_thread_ids":["child"],"status":"completed"}),
        json!({"type":"todo_list","items":[{"text":"task","completed":true}]}),
    ] {
        let state = decode(Decoder::default(), r#"{"type":"turn.started"}"#)
            .unwrap()
            .0;
        let mut item = item;
        item["id"] = json!("item");
        let (_, projection) = decode(
            state,
            &json!({"type":"item.completed","item":item}).to_string(),
        )
        .unwrap();
        assert!(matches!(
            projection.events[0],
            SessionEvent::ToolStart { .. }
        ));
        assert!(matches!(projection.events[1], SessionEvent::ToolEnd { .. }));
        assert_eq!(projection.messages.len(), 2);
    }
}

#[test]
fn unknown_events_invalid_json_and_incomplete_turns_fail() {
    for line in [
        "garbage",
        r#"{"type":"surprise"}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}"#,
    ] {
        assert!(decode(Decoder::default(), line).is_err());
    }
    let state = decode(Decoder::default(), r#"{"type":"turn.started"}"#)
        .unwrap()
        .0;
    assert!(decode(
        state,
        r#"{"type":"item.completed","item":{"id":"a","type":"future_tool"}}"#
    )
    .is_err());
}
