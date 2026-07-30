//! Image-upload round-trip contract for the web HTTP layer.
//!
//! Verifies the only gap closed by the image-upload wiring: a `PromptBody`
//! carrying `images` flows through `admit_and_drain` into `SessionInput.images`
//! and onward (existing downstream plumbing) to a persisted user `Message`
//! whose blocks contain an `Image` content block with the forwarded URL.
//!
//! Because the store exposes no read API for an *already-promoted* input
//! (`pending_inputs`/`claim_next_queue` only see unpromoted rows), the
//! authoritative assertion is at the message level: the drain lowers
//! `SessionInput.images` via `Message::user_with_images`, so a persisted
//! `Image` block proves the `images` Vec was carried from the HTTP body all
//! the way through `admit_and_drain` (the exact change under test). This is
//! race-free and a stronger end-to-end check than peeking the input row.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{Delivery, LibsqlStore, Store};

/// Fresh in-memory AppState (mirrors `web_drain_contract::state`).
async fn state() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        client_override: None,
        store,
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
    })
}

/// Seed a session row so the drain can resume it (mirrors `web_drain_contract::seed`).
async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
        })
        .await
        .unwrap();
}

/// Mock that completes a single assistant turn replying `text`.
fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Block until the drain for `sid` goes idle (no longer `draining`), mirroring
/// `web_drain_contract::wait_idle`.
async fn wait_idle(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..200 {
        let idle = state
            .handles
            .lock()
            .await
            .get(sid)
            .map(|h| !h.draining.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(true);
        if idle {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("drain for {sid} never went idle");
}

#[tokio::test]
async fn prompt_body_images_round_trip_to_persisted_message() {
    let state = state().await;
    let sid = "vision";
    seed(&state, sid).await;

    // A single tiny PNG as a data URI (one image attachment).
    let data_uri = "data:image/png;base64,iVBORw0KGgo=".to_string();

    // Admit through the production admit_and_drain, forwarding `images` — the
    // exact wiring added to PromptBody → admit_and_drain → SessionInput.images.
    let seq = opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        "describe this".into(),
        vec![data_uri.clone()],
        Delivery::Steer,
        mock_reply("seen"),
        std::env::temp_dir(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(seq > 0, "admit must return a positive seq: {seq}");

    wait_idle(&state, sid).await;

    // The drain lowers SessionInput.images into a user Message via
    // Message::user_with_images, so a persisted Image block with the forwarded
    // URL proves the images Vec survived the full HTTP round trip.
    let msgs = state.store.load_messages(sid).await.unwrap();
    let images: Vec<&ContentBlock> = msgs
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m: &Message| m.blocks.iter())
        .filter(|b| matches!(b, ContentBlock::Image { .. }))
        .collect();
    assert_eq!(
        images.len(),
        1,
        "user message must carry exactly one Image block; got {images:?}; msgs={msgs:?}"
    );
    match images[0] {
        ContentBlock::Image { url, .. } => {
            assert_eq!(
                *url, data_uri,
                "Image block URL must be the forwarded data URI"
            );
        }
        _ => unreachable!("filtered to Image blocks above"),
    }
}

/// A plain-text prompt (no `images`) must NOT produce an Image block — guards
/// against the wiring accidentally injecting images for text-only prompts.
#[tokio::test]
async fn plain_text_prompt_has_no_image_blocks() {
    let state = state().await;
    let sid = "plain";
    seed(&state, sid).await;

    let seq = opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        "hello".into(),
        Vec::new(),
        Delivery::Steer,
        mock_reply("hi"),
        std::env::temp_dir(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(seq > 0);

    wait_idle(&state, sid).await;

    let has_image = state
        .store
        .load_messages(sid)
        .await
        .unwrap()
        .iter()
        .flat_map(|m| m.blocks.iter())
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    assert!(
        !has_image,
        "a text-only prompt must not produce any Image content block"
    );
}
