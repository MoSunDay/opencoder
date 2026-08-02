//! Tests for HTTP image pre-fetching during session replay.

use super::*;
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use std::collections::HashMap;
use crate::chat::ChatBlock;
use ratatui::text::Line;
use super::replay::{prefetch_image_bytes, replay_one};

/// Build a minimal valid 2x2 red PNG as raw bytes.
fn red_png_bytes() -> Vec<u8> {
    use image::ImageEncoder;
    let img = image::RgbaImage::from_raw(
        2, 2,
        vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255],
    )
    .unwrap();
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    buf
}

#[test]
fn replay_one_renders_prefetched_http_image() {
    let url = "https://example.com/photo.png";
    let mut prefetched = HashMap::new();
    prefetched.insert(url.to_string(), red_png_bytes());

    let msg = Message {
        id: "u1".into(),
        role: Role::User,
        blocks: vec![
            ContentBlock::Text { text: "look at this".into() },
            ContentBlock::Image { url: url.into(), detail: None },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    replay_one(&mut chat, &msg, &prefetched);

    let images: Vec<_> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(images.len(), 1, "should produce one Image block");

    if let ChatBlock::Image { rendered, .. } = images[0] {
        assert!(
            !rendered.is_empty(),
            "prefetched HTTP image should render non-empty lines"
        );
    }
}

#[test]
fn replay_one_http_image_without_prefetch_is_placeholder() {
    let url = "https://example.com/missing.png";
    let empty: HashMap<String, Vec<u8>> = HashMap::new();

    let msg = Message {
        id: "u2".into(),
        role: Role::User,
        blocks: vec![
            ContentBlock::Text { text: "check this".into() },
            ContentBlock::Image { url: url.into(), detail: None },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    replay_one(&mut chat, &msg, &empty);

    let images: Vec<_> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(images.len(), 1);

    if let ChatBlock::Image { rendered, .. } = images[0] {
        assert!(
            rendered.is_empty(),
            "HTTP image without prefetch should be a placeholder"
        );
    }
}

#[test]
fn replay_one_prefetched_tool_image_renders() {
    let url = "https://example.com/screenshot.png";
    let mut prefetched = HashMap::new();
    prefetched.insert(url.to_string(), red_png_bytes());

    let msg = Message {
        id: "m-tool".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "screenshot taken".into(),
            is_error: false,
            images: vec![url.into()],
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    // Need a matching Tool block for the ToolResult to attach output.
    chat.blocks.push(ChatBlock::Tool {
        id: "t1".into(),
        header: Line::from("test"),
        output: Vec::new(),
        collapsed: false,
    });
    replay_one(&mut chat, &msg, &prefetched);

    let images: Vec<_> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(images.len(), 1, "tool image should produce an Image block");

    if let ChatBlock::Image { rendered, .. } = images[0] {
        assert!(
            !rendered.is_empty(),
            "prefetched tool image should render"
        );
    }
}

/// `prefetch_image_bytes` collects HTTP URLs but skips data URIs.
#[tokio::test]
async fn prefetch_skips_data_uris_and_collects_http() {
    // We can't actually fetch HTTP in tests, but we can verify that
    // data URIs are skipped (not attempted as network fetches).
    let data_uri = "data:image/png;base64,iVBORw0KGgo=";
    let msgs = vec![Message {
        id: "u1".into(),
        role: Role::User,
        blocks: vec![
            ContentBlock::Text { text: "img".into() },
            ContentBlock::Image { url: data_uri.into(), detail: None },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }];

    let map = prefetch_image_bytes(&msgs).await;
    // Data URIs should not appear in the map (no HTTP fetch attempted).
    assert!(map.is_empty(), "data URIs should not be prefetched");
}
