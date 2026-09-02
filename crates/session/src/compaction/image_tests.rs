use super::*;
use opencoder_core::{ContentBlock, MessageUsage};

#[test]
fn collect_head_images_gathers_user_and_tool_images() {
    let mut u = Message::user("u1", "hi");
    u.blocks.push(ContentBlock::Image {
        url: "u1.png".into(),
        detail: None,
    });
    let t = Message {
        display: None,
        id: "t1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "tc".into(),
            content: "x".into(),
            is_error: false,
            images: vec!["t1a.png".into(), "t1b.png".into()],
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };
    let imgs = collect_head_images(&[u, t]);
    assert_eq!(imgs, vec!["u1.png", "t1a.png", "t1b.png"]);
}

#[test]
fn collect_head_images_caps_at_max_keeping_most_recent() {
    let mut msgs = Vec::new();
    for i in 0..(MAX_PRESERVED_IMAGES + 2) {
        let mut m = Message::user(format!("u{i}"), "x");
        m.blocks.push(ContentBlock::Image {
            url: format!("img{i}.png"),
            detail: None,
        });
        msgs.push(m);
    }
    let imgs = collect_head_images(&msgs);
    assert_eq!(imgs.len(), MAX_PRESERVED_IMAGES);
    // newest = the last MAX_PRESERVED_IMAGES images
    assert_eq!(imgs[0], "img2.png");
    assert_eq!(
        imgs.last().unwrap(),
        &format!("img{}.png", MAX_PRESERVED_IMAGES + 1)
    );
}

#[test]
fn collect_head_images_empty_is_empty() {
    assert!(collect_head_images(&[]).is_empty());
    assert!(collect_head_images(&[Message::user("u1", "no image")]).is_empty());
}

#[test]
fn strip_images_removes_image_blocks_and_keeps_text() {
    let mut m = Message::user("u1", "hello");
    m.blocks.push(ContentBlock::Image {
        url: "x.png".into(),
        detail: None,
    });
    let stripped = strip_images(&[m]);
    assert_eq!(stripped.len(), 1);
    assert!(
        !stripped[0]
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. })),
        "Image blocks must be stripped"
    );
    assert!(!stripped[0].has_image());
    assert!(stripped[0].text().contains("hello"));
}

#[test]
fn strip_images_clears_tool_result_images() {
    let m = Message {
        display: None,
        id: "t1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "tc".into(),
            content: "shot".into(),
            is_error: false,
            images: vec!["shot.png".into()],
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };
    let stripped = strip_images(&[m]);
    match &stripped[0].blocks[0] {
        ContentBlock::ToolResult {
            images, content, ..
        } => {
            assert!(images.is_empty(), "tool images must be cleared");
            assert_eq!(content, "shot");
        }
        other => panic!("unexpected block: {other:?}"),
    }
}

/// RC4: images in the compacted head must survive compaction by attaching
/// to the summary message, so the (vision-capable) main model still sees
/// them after summarization. Deterministic, zero-network.
#[tokio::test]
async fn compaction_preserves_head_images_on_summary_message() {
    use std::sync::Arc;

    use opencoder_core::resolve_agent;
    use opencoder_core::Config;
    use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};

    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("summary of talk".into()),
        LlmEvent::Completed {
            text: "summary of talk".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 1,
                total_tokens: 2,
                ..Default::default()
            }),
        },
    ]));
    let agent = resolve_agent("act").expect("act agent");
    let mut s = SessionState::new(
        "compact-img",
        agent,
        Config {
            model: "main/glm-5.2".into(),
            ..Config::default()
        },
        mock,
        std::env::temp_dir(),
    );
    // Two turns; the head (u1+a1) carries an image, the tail (u2+a2) does not.
    let mut u1 = Message::user("u1", "look at this");
    u1.blocks.push(ContentBlock::Image {
        url: "data:image/png;base64,AAAA".into(),
        detail: None,
    });
    s.messages.push(u1);
    s.messages.push(Message::assistant("a1"));
    s.messages.push(Message::user("u2", "second"));
    s.messages.push(Message::assistant("a2"));

    let mut events: Vec<SessionEvent> = Vec::new();
    let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;
    assert!(outcome.is_ok(), "compaction must succeed: {outcome:?}");

    // The summary message (now messages[0]) must carry the preserved image.
    assert!(!s.messages.is_empty());
    let summary = &s.messages[0];
    assert_eq!(summary.role, Role::User);
    assert!(
        summary.text().starts_with("[Conversation summary so far]"),
        "summary text prefix intact"
    );
    assert!(
        summary.has_image(),
        "summary message must preserve the head image"
    );
    // And lowering yields a legal multimodal user turn with image_url.
    let lowered = opencoder_llm::lower_messages(&s.messages);
    let user_img = lowered
        .iter()
        .find(|m| m["role"] == "user" && m["content"].is_array())
        .expect("a lowered user message carrying an image");
    let content = user_img["content"].as_array().unwrap();
    assert!(
        content
            .iter()
            .any(|p| p["type"] == "image_url"
                && p["image_url"]["url"] == "data:image/png;base64,AAAA")
    );
}
