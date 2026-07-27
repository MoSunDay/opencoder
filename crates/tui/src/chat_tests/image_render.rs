use super::super::*;

/// Minimal valid 1×1 PNG as a data URI for tests.
fn tiny_png_data_uri() -> String {
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==".into()
}

#[test]
fn tool_end_with_images_renders_image_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-img".into(),
        name: "view_image".into(),
        input: serde_json::json!({"path": "cat.png"}),
    });
    let uri = tiny_png_data_uri();
    v.apply(&SessionEvent::ToolEnd {
        id: "t-img".into(),
        name: "view_image".into(),
        output: "Loaded image: cat.png (0.1 KiB)".into(),
        is_error: false,
        images: vec![uri],
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(
        images.len(),
        1,
        "expected exactly one Image block after ToolEnd with one image"
    );
    if let ChatBlock::Image { filename, .. } = images[0] {
        assert!(
            filename.contains("cat.png") || !filename.is_empty(),
            "image block should carry a display filename"
        );
    }
}

#[test]
fn tool_end_with_multiple_images_renders_all() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-multi".into(),
        name: "view_image".into(),
        input: serde_json::json!({"path": "shot.png"}),
    });
    let uri = tiny_png_data_uri();
    v.apply(&SessionEvent::ToolEnd {
        id: "t-multi".into(),
        name: "view_image".into(),
        output: "done".into(),
        is_error: false,
        images: vec![uri.clone(), uri.clone(), uri],
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(
        images.len(),
        3,
        "three images must produce three Image blocks"
    );
}

#[test]
fn tool_end_without_images_no_image_block() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ToolStart {
        id: "t-text".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo hi"}),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t-text".into(),
        name: "bash".into(),
        output: "hi".into(),
        is_error: false,
        images: Vec::new(),
    });
    let images: Vec<_> = v
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert!(
        images.is_empty(),
        "ToolEnd without images must not create Image blocks"
    );
}
