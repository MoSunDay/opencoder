//! Tests for HTTP image pre-fetching during session replay.

use super::replay::{prefetch_image_bytes, replay_one};
use super::*;
use crate::chat::ChatBlock;
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use ratatui::text::Line;
use std::collections::HashMap;

/// Build a minimal valid 2x2 red PNG as raw bytes.
fn red_png_bytes() -> Vec<u8> {
    use image::ImageEncoder;
    let img = image::RgbaImage::from_raw(
        2,
        2,
        vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ],
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
            ContentBlock::Text {
                text: "look at this".into(),
            },
            ContentBlock::Image {
                url: url.into(),
                detail: None,
            },
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
            ContentBlock::Text {
                text: "check this".into(),
            },
            ContentBlock::Image {
                url: url.into(),
                detail: None,
            },
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
    // Need a matching StepGroup call for the ToolResult to attach output.
    chat.blocks.push(ChatBlock::StepGroup {
        steps: vec![crate::chat::Step {
            thinking: Vec::new(),
            calls: vec![crate::chat::ToolCall {
                id: "t1".into(),
                header: Line::from("test"),
                output: Vec::new(),
                started_at_ms: Some(0),
                elapsed_ms: Some(0),
                expanded: false,
            }],
            open: false,
        }],
        open: false,
    });
    replay_one(&mut chat, &msg, &prefetched);

    let images: Vec<_> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Image { .. }))
        .collect();
    assert_eq!(images.len(), 1, "tool image should produce an Image block");

    if let ChatBlock::Image { rendered, .. } = images[0] {
        assert!(!rendered.is_empty(), "prefetched tool image should render");
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
            ContentBlock::Image {
                url: data_uri.into(),
                detail: None,
            },
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

// ── concurrency / budget (injected fake fetcher, no network) ─────────────

use std::sync::atomic::{AtomicUsize, Ordering};

use super::replay::prefetch_image_bytes_with;

fn http_msg(urls: &[&str]) -> Vec<Message> {
    urls.iter()
        .map(|u| Message {
            id: format!("m-{u}"),
            role: Role::User,
            blocks: vec![
                ContentBlock::Text {
                    text: "imgs".into(),
                },
                ContentBlock::Image {
                    url: (*u).into(),
                    detail: None,
                },
            ],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        })
        .collect()
}

/// Fetches run concurrently: 3 URLs each sleeping 80ms finish in roughly one
/// slot (~80ms), not the serial 240ms, and at least two are in flight at the
/// same time.
#[tokio::test]
async fn prefetch_fetches_run_concurrently() {
    let inflight = std::sync::Arc::new(AtomicUsize::new(0));
    let max_inflight = std::sync::Arc::new(AtomicUsize::new(0));
    let map = prefetch_image_bytes_with(
        &http_msg(&[
            "https://example.com/a.png",
            "https://example.com/b.png",
            "https://example.com/c.png",
        ]),
        {
            let inflight = inflight.clone();
            let max_inflight = max_inflight.clone();
            move |url: String| {
                let inflight = inflight.clone();
                let max_inflight = max_inflight.clone();
                async move {
                    let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_inflight.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    Some(vec![1u8; url.len()])
                }
            }
        },
        std::time::Duration::from_secs(8),
    )
    .await;

    assert_eq!(map.len(), 3, "all three fetches must resolve");
    assert!(
        max_inflight.load(Ordering::SeqCst) >= 2,
        "fetches must overlap; max in-flight was {}",
        max_inflight.load(Ordering::SeqCst)
    );
}

/// The overall budget bounds the batch: one hung host cannot stall the
/// rebuild; the fast fetch that already arrived is still returned.
#[tokio::test]
async fn prefetch_budget_returns_partial_success_and_bounds_wait() {
    let start = std::time::Instant::now();
    let map = prefetch_image_bytes_with(
        &http_msg(&[
            "https://fast.example.com/ok.png",
            "https://slow.example.com/hang.png",
        ]),
        |url: String| async move {
            if url.contains("fast") {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Some(vec![2u8])
            } else {
                // Simulates a host that never answers (well past the budget).
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                Some(vec![9u8])
            }
        },
        std::time::Duration::from_millis(150),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(
        map.contains_key("https://fast.example.com/ok.png"),
        "fast fetch must survive the budget cut"
    );
    assert!(
        !map.contains_key("https://slow.example.com/hang.png"),
        "hung fetch must be aborted at the budget deadline"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "budget must bound total wait; took {elapsed:?}"
    );
}

/// An empty URL set never spawns tasks and returns instantly (data URIs are
/// handled inline by `replay_one`, not here).
#[tokio::test]
async fn prefetch_empty_url_set_is_free() {
    let map = prefetch_image_bytes_with(
        &http_msg(&["data:image/png;base64,AAAA"]),
        |url: String| async move { Some(vec![url.len() as u8]) },
        std::time::Duration::from_secs(8),
    )
    .await;
    assert!(map.is_empty(), "data URIs must not be fetched");
}
