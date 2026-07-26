//! Image-carrying `ToolOutput`: `ok_with_images` constructs an output with
//! attachments, and the `images` field is serde-backward-compatible (old
//! serialized rows without the key still deserialize to an empty vec).

use opencoder_core::ToolOutput;

#[test]
fn ok_with_images_carries_the_images() {
    let out = ToolOutput::ok_with_images(
        "screenshot captured",
        vec![
            "data:image/png;base64,iVBOR=".into(),
            "https://x.test/a.png".into(),
        ],
    );
    assert!(!out.is_error, "ok_with_images must not be an error");
    assert_eq!(out.content, "screenshot captured");
    assert_eq!(out.images.len(), 2, "both images must be carried");
    assert_eq!(out.images[0], "data:image/png;base64,iVBOR=");
    assert_eq!(out.images[1], "https://x.test/a.png");
}

#[test]
fn ok_and_err_produce_empty_images() {
    let ok = ToolOutput::ok("done");
    assert!(ok.images.is_empty(), "ok() must default to no images");
    assert!(!ok.is_error);

    let err = ToolOutput::err("boom");
    assert!(err.images.is_empty(), "err() must default to no images");
    assert!(err.is_error);
}

#[test]
fn tool_output_with_images_roundtrips_through_serde() {
    let out = ToolOutput::ok_with_images("see attached", vec!["data:image/png;base64,YQ==".into()]);
    let json = serde_json::to_string(&out).unwrap();
    let back: ToolOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(back.content, "see attached");
    assert!(!back.is_error);
    assert_eq!(back.images.len(), 1, "image must survive the round-trip");
    assert_eq!(back.images[0], "data:image/png;base64,YQ==");
}

#[test]
fn tool_output_without_images_serializes_with_empty_images() {
    // A text-only output still serializes the (empty) images key.
    let out = ToolOutput::ok("plain");
    let v = serde_json::to_value(&out).unwrap();
    assert!(v["images"].is_array(), "images key must be present");
    assert!(v["images"].as_array().unwrap().is_empty());
}

#[test]
fn old_tool_output_json_without_images_still_deserializes() {
    // Backward compatibility: a persisted ToolOutput blob from before the
    // `images` field existed (no `images` key) must deserialize cleanly into
    // an output with an empty images vec — `#[serde(default)]`.
    let legacy = r#"{"content":"ok","is_error":false}"#;
    let back: ToolOutput = serde_json::from_str(legacy).unwrap();
    assert_eq!(back.content, "ok");
    assert!(!back.is_error);
    assert!(
        back.images.is_empty(),
        "missing images key must default to empty"
    );
}
