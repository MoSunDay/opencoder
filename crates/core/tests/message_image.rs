//! Multimodal content model: `ContentBlock::Image` serde shape, `has_image`,
//! `user_with_images`, and that images contribute to `estimate_chars` (so
//! compaction thresholds account for vision attachments).

use opencoder_core::{ContentBlock, Message};

#[test]
fn image_block_serializes_with_image_tag() {
    let block = ContentBlock::Image {
        url: "data:image/png;base64,iVBOR=".into(),
        detail: Some("high".into()),
    };
    let v = serde_json::to_value(&block).unwrap();
    // The enum uses `tag = "kind"`, so the discriminator is "image".
    assert_eq!(v["kind"], "image", "image discriminator must be 'image'");
    assert_eq!(v["url"], "data:image/png;base64,iVBOR=");
    assert_eq!(v["detail"], "high");
}

#[test]
fn image_block_roundtrips_with_and_without_detail() {
    for detail in [Some("low"), None] {
        let block = ContentBlock::Image {
            url: "https://example.com/a.png".into(),
            detail: detail.map(str::to_string),
        };
        let json = serde_json::to_string(&block).unwrap();
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ContentBlock::Image { url, detail: d } => {
                assert_eq!(url, "https://example.com/a.png");
                assert_eq!(d.as_deref(), detail);
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }
}

#[test]
fn has_image_detects_image_blocks() {
    let plain = Message::user("u1", "hello");
    assert!(!plain.has_image());

    let with_img = Message::user_with_images(
        "u2",
        "describe this",
        &["data:image/png;base64,YQ==".to_string()],
    );
    assert!(with_img.has_image());
    // text() must exclude image data so plain-text views stay clean.
    assert_eq!(with_img.text(), "describe this");
}

#[test]
fn user_with_images_appends_one_image_block_per_uri() {
    let m = Message::user_with_images(
        "u3",
        "p",
        &[
            "data:image/png;base64,YQ==".to_string(),
            "https://x.test/b.jpg".to_string(),
        ],
    );
    let images: Vec<_> = m.blocks.iter().filter_map(|b| b.as_image()).collect();
    assert_eq!(images.len(), 2, "one Image block per uri");
    assert_eq!(images[0].0, "data:image/png;base64,YQ==");
    assert_eq!(images[1].0, "https://x.test/b.jpg");
}

#[test]
fn estimate_chars_counts_image_attachment_without_dumping_base64() {
    // A huge base64 payload must NOT be counted literally (it would dwarf
    // compaction budgets); instead a fixed per-image cost is added.
    let big = format!("data:image/png;base64,{}", "A".repeat(200_000));
    let m = Message::user_with_images("u4", "x", &[big]);
    let chars = m.estimate_chars();
    // Fixed cost (~1024) + the short text "x"; far less than the 200k payload.
    assert!(
        chars.len() > 1024 && chars.len() < 5_000,
        "image estimate must be a fixed rough cost, got {} chars",
        chars.len()
    );
}

#[test]
fn old_blocks_json_without_image_still_deserializes() {
    // A pre-image blocks blob (Text + ToolResult) must round-trip unchanged:
    // adding the Image variant cannot break existing persisted transcripts.
    let legacy = r#"[{"kind":"text","text":"hi"},{"kind":"tool_result","tool_use_id":"c1","content":"ok","is_error":false}]"#;
    let blocks: Vec<ContentBlock> = serde_json::from_str(legacy).unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(blocks
        .iter()
        .all(|b| !matches!(b, ContentBlock::Image { .. })));
}

// --- tool-result images ---

#[test]
fn tool_result_with_images_serializes_the_images_key() {
    let block = ContentBlock::ToolResult {
        tool_use_id: "t1".into(),
        content: "see image".into(),
        is_error: false,
        images: vec!["data:image/png;base64,iVBOR=".into()],
    };
    let v = serde_json::to_value(&block).unwrap();
    assert_eq!(
        v["kind"], "tool_result",
        "discriminator must be tool_result"
    );
    assert_eq!(v["tool_use_id"], "t1");
    assert_eq!(v["content"], "see image");
    assert_eq!(v["is_error"], false);
    assert_eq!(
        v["images"][0], "data:image/png;base64,iVBOR=",
        "image URI must be serialized"
    );
}

#[test]
fn tool_result_without_images_omits_the_images_key() {
    // skip_serializing_if = "Vec::is_empty": a text-only ToolResult must NOT
    // emit an `images` key, keeping persisted blobs byte-identical to the
    // pre-image shape.
    let block = ContentBlock::ToolResult {
        tool_use_id: "t1".into(),
        content: "just text".into(),
        is_error: false,
        images: Vec::new(),
    };
    let v = serde_json::to_value(&block).unwrap();
    assert!(
        v.get("images").is_none(),
        "empty images must be skipped, got: {v}"
    );
}

#[test]
fn old_tool_result_json_without_images_deserializes_to_empty() {
    // Backward compatibility: a persisted ToolResult from before `images`
    // existed (no key) must deserialize to an empty images vec.
    let legacy = r#"{"kind":"tool_result","tool_use_id":"c1","content":"ok","is_error":false}"#;
    let back: ContentBlock = serde_json::from_str(legacy).unwrap();
    match back {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            images,
        } => {
            assert_eq!(tool_use_id, "c1");
            assert_eq!(content, "ok");
            assert!(!is_error);
            assert!(
                images.is_empty(),
                "missing images key must default to empty"
            );
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn tool_result_images_roundtrip() {
    let block = ContentBlock::ToolResult {
        tool_use_id: "t1".into(),
        content: "two screenshots".into(),
        is_error: true,
        images: vec![
            "data:image/png;base64,YQ==".into(),
            "https://x.test/b.jpg".into(),
        ],
    };
    let json = serde_json::to_string(&block).unwrap();
    let back: ContentBlock = serde_json::from_str(&json).unwrap();
    match back {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
            images,
        } => {
            assert_eq!(tool_use_id, "t1");
            assert_eq!(content, "two screenshots");
            assert!(is_error);
            assert_eq!(images.len(), 2, "both images must round-trip");
            assert_eq!(images[0], "data:image/png;base64,YQ==");
            assert_eq!(images[1], "https://x.test/b.jpg");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn estimate_chars_counts_tool_result_images_at_fixed_cost() {
    // A tool result with an image attachment must contribute a fixed rough
    // cost (~1024 chars) to estimate_chars — NOT the full base64 payload,
    // which would dwarf compaction budgets.
    let huge = format!("data:image/png;base64,{}", "A".repeat(200_000));
    let m = Message {
        display: None,
        id: "m1".into(),
        role: opencoder_core::Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "shot".into(),
            is_error: false,
            images: vec![huge],
        }],
        model: None,
        agent: None,
        usage: opencoder_core::MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };
    let chars = m.estimate_chars();
    // "shot" (4) + fixed image cost (1024) + newline; far below the 200k payload.
    assert!(
        chars.len() > 1024 && chars.len() < 5_000,
        "tool-result image estimate must be a fixed rough cost, got {} chars",
        chars.len()
    );
}
