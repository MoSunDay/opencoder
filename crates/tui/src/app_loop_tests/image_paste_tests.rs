//! route_paste image-classification + chunked-upload tests.

use super::*;

/// Plain text is inserted verbatim when no modal owns the paste.
#[test]
fn route_paste_into_main_composer_inserts_verbatim_text() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let flow = route_paste(
        "plain text",
        false,
        false,
        false,
        false,
        &mut model_menu,
        &mut None,
        &mut None,
        &mut command_menu,
        &mut None,
        &mut input,
        &mut idx,
        &mut pending_images,
        &mut asm,
        &mut chat,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(input, "plain text");
    assert_eq!(idx, "plain text".chars().count());
}

/// A picker without a text field owns and swallows the paste.
#[test]
fn route_paste_swallowed_when_task_picker_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::new();
    let mut idx = 0usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let flow = route_paste(
        "plain text",
        true,
        false,
        false,
        false,
        &mut model_menu,
        &mut None,
        &mut None,
        &mut command_menu,
        &mut None,
        &mut input,
        &mut idx,
        &mut pending_images,
        &mut asm,
        &mut chat,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert!(input.is_empty());
    assert_eq!(idx, 0);
}

/// Cache-salt modal isolation preserves existing composer state.
#[test]
fn route_paste_swallowed_when_cache_salt_menu_open() {
    let mut model_menu: Option<ModelMenu> = None;
    let mut command_menu: Option<CommandMenu> = None;
    let mut input = String::from("kept");
    let mut idx = 2usize;
    let mut pending_images: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let flow = route_paste(
        "plain text",
        false,
        true,
        false,
        false,
        &mut model_menu,
        &mut None,
        &mut None,
        &mut command_menu,
        &mut None,
        &mut input,
        &mut idx,
        &mut pending_images,
        &mut asm,
        &mut chat,
        Path::new("."),
    );
    assert!(matches!(flow, LoopFlow::Redraw));
    assert_eq!(input, "kept");
    assert_eq!(idx, 2);
}

/// Drive one paste through `route_paste` with no modal open, mirroring the
/// main-composer path in `app.rs`.
fn paste(
    pasted: &str,
    pending: &mut Vec<(String, String)>,
    asm: &mut crate::image_chunk::Assembly,
    chat: &mut ChatView,
    input: &mut String,
    idx: &mut usize,
) -> LoopFlow {
    let mut mm: Option<ModelMenu> = None;
    let mut cm: Option<CommandMenu> = None;
    route_paste(
        pasted,
        false,
        false,
        false,
        false,
        &mut mm,
        &mut None,
        &mut None,
        &mut cm,
        &mut None,
        input,
        idx,
        pending,
        asm,
        chat,
        Path::new("."),
    )
}

/// A data-URI paste attaches verbatim (trailing terminal newline stripped)
/// and never touches the composer text.
#[test]
fn paste_data_uri_attaches_verbatim() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(
        pending,
        vec![(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==".to_string(),
            "pasted.png".to_string()
        )]
    );
    assert!(input.is_empty());
}

/// An image URL paste attaches the URL verbatim with the last path segment
/// as its label.
#[test]
fn paste_image_url_attaches() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "https://example.com/pics/cat.jpeg",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, "https://example.com/pics/cat.jpeg");
    assert_eq!(pending[0].1, "cat.jpeg");
    assert!(input.is_empty());
}

/// A non-image URL is plain text: inserted into the composer, nothing
/// attached.
#[test]
fn paste_non_image_url_inserts_text() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "https://example.com/docs\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(pending.is_empty());
    assert!(input.contains("https://example.com/docs"));
}

/// A `data:text/plain` URI is not an image: inserted verbatim.
#[test]
fn paste_data_text_plain_inserts_text() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "data:text/plain;base64,aGVsbG8=",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(pending.is_empty());
    assert_eq!(input, "data:text/plain;base64,aGVsbG8=");
}

/// Pasting an existing image file's path reads + encodes it as a data URI.
#[test]
fn paste_local_path_attaches() {
    let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
    std::fs::write(tmp.path(), png_bytes).unwrap();
    let path_str = tmp.path().to_str().unwrap().to_string();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        &path_str,
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(pending.len(), 1);
    assert!(pending[0].0.starts_with("data:image/png;base64,"));
    assert!(input.is_empty());
}

/// A whole chunked upload in one atomic paste: all frames consumed, one
/// reassembled attachment, composer untouched, accumulator drained.
#[test]
fn paste_chunk_block_single_shot() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "ocimg begin t1 png 3\nocimg chunk t1 0 AAA\nocimg chunk t1 1 BBB\nocimg chunk t1 2 CCC\nocimg end t1\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(
        pending,
        vec![(
            "data:image/png;base64,AAABBBCCC".to_string(),
            "pasted.png (3 chunks)".to_string()
        )]
    );
    assert!(input.is_empty());
    assert!(asm.is_empty(), "completed assembly is removed from state");
}

/// Frames arriving across separate pastes accumulate until `end` completes
/// the assembly.
#[test]
fn paste_chunk_frames_incremental() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    paste(
        "ocimg begin t2 jpeg 2\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(pending.is_empty());
    paste(
        "ocimg chunk t2 0 AAA\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(pending.is_empty());
    paste(
        "ocimg chunk t2 1 BBB\nocimg end t2\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert_eq!(
        pending,
        vec![(
            "data:image/jpeg;base64,AAABBB".to_string(),
            "pasted.jpeg (2 chunks)".to_string()
        )]
    );
    assert!(input.is_empty());
}

/// Out-of-order chunks reassemble in sequence order, not arrival order.
#[test]
fn paste_chunk_out_of_order() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    for frame in [
        "ocimg begin t3 png 2\n",
        "ocimg chunk t3 1 BBB\n",
        "ocimg chunk t3 0 AAA\n",
        "ocimg end t3\n",
    ] {
        paste(
            frame,
            &mut pending,
            &mut asm,
            &mut chat,
            &mut input,
            &mut idx,
        );
    }
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, "data:image/png;base64,AAABBB");
    assert!(input.is_empty());
}

/// An `end` with a missing chunk warns and drops the assembly: nothing
/// attached and no frame line leaks into the composer.
#[test]
fn paste_chunk_missing_piece_warns() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    for frame in [
        "ocimg begin t5 png 2\n",
        "ocimg chunk t5 0 AAA\n",
        "ocimg end t5\n",
    ] {
        paste(
            frame,
            &mut pending,
            &mut asm,
            &mut chat,
            &mut input,
            &mut idx,
        );
    }
    assert!(pending.is_empty());
    assert!(input.is_empty(), "frame lines never reach the composer");
}

/// A huge single-line text paste takes the plain-text path with no decode
/// work: inserted verbatim, nothing attached.
#[test]
fn paste_random_long_text_not_blocked() {
    let blob = "x".repeat(200_000);
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        &blob,
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(pending.is_empty());
    assert_eq!(input, blob);
}

/// Frames and text mixed in one paste: frames attach an image, the leftover
/// line lands in the composer.
#[test]
fn paste_mixed_frames_and_text() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    let flow = paste(
        "ocimg begin t4 png 1\nocimg chunk t4 0 AAA\nocimg end t4\nhello",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(matches!(flow, LoopFlow::Proceed));
    assert_eq!(pending.len(), 1);
    assert_eq!(input, "hello");
}

// ----- Attachment marker (📎) consistency tests -----

/// A helper that collects all marker block text from a ChatView.
fn marker_texts(chat: &ChatView) -> Vec<String> {
    use crate::chat::ChatBlock;
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => {
                let text: String = lines
                    .iter()
                    .flat_map(|l| l.spans.iter())
                    .map(|s| s.content.to_string())
                    .collect();
                Some(text)
            }
            _ => None,
        })
        .collect()
}

/// A data-URI image paste must produce a green `📎` marker in the chat stream.
#[test]
fn paste_data_uri_emits_attach_marker() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    paste(
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==\n",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    let markers = marker_texts(&chat);
    assert!(
        markers.iter().any(|m| m.contains('\u{1f4ce}')),
        "expected a clip marker, got: {:?}",
        markers
    );
}

/// An image URL paste must produce a `📎` marker.
#[test]
fn paste_image_url_emits_attach_marker() {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    paste(
        "https://example.com/pics/cat.jpeg",
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    let markers = marker_texts(&chat);
    assert!(
        markers.iter().any(|m| m.contains('\u{1f4ce}')),
        "expected a clip marker, got: {:?}",
        markers
    );
}

/// A file-path image paste must produce a `📎` marker with the filename.
#[test]
fn paste_file_path_emits_attach_marker() {
    let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
    std::fs::write(tmp.path(), png_bytes).unwrap();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    paste(
        tmp.path().to_str().unwrap(),
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    let markers = marker_texts(&chat);
    assert!(
        markers
            .iter()
            .any(|m| m.contains('\u{1f4ce}') && m.contains(".png")),
        "expected a clip marker with .png, got: {:?}",
        markers
    );
}

/// A quoted file path must still load as an image attachment with a marker.
#[test]
fn paste_quoted_file_path_emits_attach_marker() {
    let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
    std::fs::write(tmp.path(), png_bytes).unwrap();
    let quoted = format!("\"{}\"", tmp.path().to_str().unwrap());
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut asm = crate::image_chunk::Assembly::new();
    let mut chat = ChatView::default();
    let mut input = String::new();
    let mut idx = 0usize;
    paste(
        &quoted,
        &mut pending,
        &mut asm,
        &mut chat,
        &mut input,
        &mut idx,
    );
    assert!(!pending.is_empty(), "quoted path should attach as image");
    let markers = marker_texts(&chat);
    assert!(
        markers.iter().any(|m| m.contains('\u{1f4ce}')),
        "expected a clip marker, got: {:?}",
        markers
    );
}
