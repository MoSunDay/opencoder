use super::support::*;
use opencoder_core::{ContentBlock, Message, ProviderState};
use opencoder_llm::{
    responses::request::{lower_input, to_body},
    RequestPurpose,
};
use serde_json::json;

fn scope() -> ProviderState {
    ProviderState {
        provider: "fixture".into(),
        base_url: "http://localhost".into(),
        model: "gpt-6-astra".into(),
        output: vec![],
    }
}

#[test]
fn maps_parameters_and_preserves_optional_tool_fields() {
    let mut req = request();
    req.model = "gpt-6-astra".into();
    req.max_tokens = Some(2000);
    req.temperature = Some(0.3);
    req.cache_salt = Some("act:session".into());
    req.tools = vec![
        json!({"type":"function", "function":{"name":"read", "parameters":{
            "type":"object", "properties":{"path":{"type":"string"},"limit":{"type":"integer"}}, "required":["path"]
        }}}),
    ];
    let body = to_body(&req, &scope()).unwrap();
    assert_eq!(body["max_output_tokens"], 2000);
    assert_eq!(body["reasoning"], json!({"effort":"high","summary":"auto"}));
    assert_eq!(body["tools"][0]["strict"], false);
    assert_eq!(body["tools"][0]["parameters"]["required"], json!(["path"]));
    assert_eq!(body["prompt_cache_key"], "act:session");
    assert_eq!(body["store"], false);
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    for key in [
        "messages",
        "stream_options",
        "max_tokens",
        "reasoning_effort",
        "temperature",
        "cache_salt",
    ] {
        assert!(body.get(key).is_none(), "{key}");
    }
    req.tools[0]["function"]["strict"] = json!(true);
    assert_eq!(to_body(&req, &scope()).unwrap()["tools"][0]["strict"], true);
    req.tools[0]["type"] = json!("web_search");
    assert!(to_body(&req, &scope()).is_err());
}

#[test]
fn auxiliary_responses_have_reasoning_budget() {
    for purpose in [RequestPurpose::Title, RequestPurpose::Verify] {
        let mut req = request();
        req.purpose = purpose;
        req.max_tokens = Some(8);
        let body = to_body(&req, &scope()).unwrap();
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["reasoning"]["effort"], "low");
    }
}

#[test]
fn state_replay_preserves_phase_ids_and_ciphertext_without_duplicate_blocks() {
    let output = vec![reasoning(), call("call_1", "read", json!({"path":"a"}))];
    let mut message = Message::assistant("a");
    message.blocks = vec![
        ContentBlock::Reasoning {
            text: "display summary".into(),
        },
        ContentBlock::Text {
            text: "display normalized".into(),
        },
        ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "read".into(),
            input: json!({"path":"a"}),
        },
    ];
    message.provider_state = Some(ProviderState {
        output: output.clone(),
        ..scope()
    });
    assert_eq!(lower_input(&[message.clone()], &scope()).unwrap(), output);
    let chat = opencoder_llm::lower_messages(std::slice::from_ref(&message));
    assert_eq!(chat[0]["content"], "display normalized");
    assert_eq!(chat[0]["tool_calls"][0]["id"], "call_1");
    assert!(chat[0].get("reasoning_content").is_none());
    assert!(!serde_json::to_string(&chat)
        .unwrap()
        .contains("opaque-fixture"));
    let mut other = scope();
    other.model = "gpt-5".into();
    let lowered = lower_input(&[message.clone()], &other).unwrap();
    assert_eq!(lowered[0]["content"], "display normalized");
    assert_eq!(lowered[1]["call_id"], "call_1");
    assert!(lowered[1].get("id").is_none());
    assert!(!serde_json::to_string(&lowered)
        .unwrap()
        .contains("opaque-fixture"));
    message
        .blocks
        .retain(|b| !matches!(b, ContentBlock::ToolUse { .. }));
    assert_eq!(
        lower_input(&[message], &scope()).unwrap().len(),
        1,
        "filtered calls must not resurrect"
    );
}

#[test]
fn images_and_error_results_keep_their_call_association() {
    let mut result = Message::user_with_images("u", "look", &["data:image/png;base64,AA==".into()]);
    result.blocks.insert(
        0,
        ContentBlock::ToolResult {
            tool_use_id: "call_1".into(),
            content: "not found".into(),
            is_error: true,
            images: vec!["https://example.test/tool.png".into()],
        },
    );
    let input = lower_input(&[Message::system("s", "rules"), result], &scope()).unwrap();
    assert_eq!(input[0]["role"], "system");
    assert_eq!(input[1]["type"], "function_call_output");
    assert_eq!(input[1]["call_id"], "call_1");
    assert_eq!(input[1]["output"][0]["text"], "[error] not found");
    assert_eq!(input[1]["output"][1]["type"], "input_image");
    assert_eq!(
        input[2]["content"][1]["image_url"],
        "data:image/png;base64,AA=="
    );
}
