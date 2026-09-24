use super::*;
use serde_json::json;

fn event(state: Decoder, value: Value) -> Result<(Decoder, Projection)> {
    decode(state, &value.to_string())
}
fn thread() -> Decoder {
    event(
        Decoder::default(),
        json!({"type":"thread.started","thread_id":"fixture"}),
    )
    .unwrap()
    .0
}
fn warning(kind: &str) -> Value {
    json!({"type":kind,"item":{"id":"item_0","type":"error","message":"configuration ignored"}})
}

#[test]
fn real_cli_startup_notifications_preserve_status_without_starting_turn() {
    let raw = include_str!("fixtures/startup-notifications.jsonl");
    let mut state = Decoder {
        prefix: "test".into(),
        ..Default::default()
    };
    let mut statuses = 0;
    for (index, line) in raw.lines().enumerate() {
        let (next, out) = decode(state, line).unwrap();
        state = next;
        for event in out.events {
            if let SessionEvent::Status(text) = event {
                statuses += 1;
                assert!(text.contains("auth_provider"));
                assert!(text.contains("test:item_"));
            }
        }
        if index < 3 {
            assert!(!state.started);
            assert!(!state.completed);
            assert!(out.messages.is_empty());
        }
    }
    assert_eq!(statuses, 2);
    assert!(state.completed);
    assert!(state.failed.is_none());
    assert_eq!(state.usage.output_tokens, 5);
}

#[test]
fn notification_needs_thread_complete_item_and_valid_fields() {
    assert!(event(Decoder::default(), warning("item.completed")).is_err());
    for kind in ["item.started", "item.updated"] {
        assert!(event(thread(), warning(kind)).is_err());
    }
    for key in ["id", "message"] {
        let mut value = warning("item.completed");
        value["item"].as_object_mut().unwrap().remove(key);
        assert!(event(thread(), value).is_err());
    }
}

#[test]
fn tools_and_agent_text_still_require_active_turn() {
    for kind in ["agent_message", "reasoning", "command_execution"] {
        assert!(event(thread(),json!({"type":"item.completed","item":{"id":"x","type":kind,"text":"bad","command":"bad"}})).is_err());
    }
    let state = Decoder {
        completed: true,
        ..thread()
    };
    assert!(event(state, warning("item.completed")).is_err());
}

#[test]
fn fatal_errors_are_not_converted_to_startup_notifications() {
    for value in [
        json!({"type":"error","message":"fatal"}),
        json!({"type":"turn.failed","error":{"message":"fatal"}}),
    ] {
        let (state, out) = event(thread(), value).unwrap();
        assert_eq!(state.failed.as_deref(), Some("fatal"));
        assert!(matches!(&out.events[0],SessionEvent::Error(e) if e=="fatal"));
        assert!(event(state, warning("item.completed")).is_err());
    }
}
