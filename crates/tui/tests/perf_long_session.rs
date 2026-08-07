//! Performance regression guard for long-session rendering (A1).
//!
//! Verifies that viewport virtualization keeps per-frame cost O(visible_h)
//! regardless of transcript length. Without virtualization, every frame
//! re-flattened the entire transcript and re-computed wrapped-row counts —
//! O(n) growing with session length. With the `ViewportCache`, flattening and
//! row-counting happen once per body-refresh cycle and each frame only clones
//! the visible lines via an O(log n) binary search over the cumulative row
//! table.

use std::time::Instant;

use opencoder_tui::chat::{ChatBlock, ChatView};
use opencoder_tui::render_viewport::ViewportCache;
use ratatui::text::Line;

/// Build a `ChatView` simulating a realistic long transcript: each iteration
/// appends a user-prompt `Marker` block plus a completed (`done`) `Assistant`
/// block with 4 rendered body lines of moderate length. `n` iterations →
/// `2 * n` blocks. The body lines are wide enough to wrap at width 80 so the
/// wrapped-row accounting in `ViewportCache` is exercised, not just the raw
/// line count.
fn build_large_chat(n: usize) -> ChatView {
    let mut blocks = Vec::with_capacity(n * 2);
    for i in 0..n {
        blocks.push(ChatBlock::Marker(vec![Line::from(format!(
            "user: this is prompt number {i} with enough context to feel realistic"
        ))]));
        blocks.push(ChatBlock::Assistant {
            raw: String::new(),
            rendered: vec![
                Line::from(format!(
                    "Here is my response to prompt {i}. Let me walk through the approach."
                )),
                Line::from(
                    "First we map the requirement onto the existing module boundaries so the \
                     change stays localized and easy to review.",
                ),
                Line::from(
                    "Then we implement the core logic, covering the happy path as well as the \
                     obvious edge cases and failure modes.",
                ),
                Line::from(
                    "Finally we add tests that pin the behaviour down and guard against future \
                     regressions in this area of the codebase.",
                ),
            ],
            done: true,
        });
    }
    ChatView {
        blocks,
        ..Default::default()
    }
}

/// Build a cache for an `n`-block transcript at width 80 and measure the cost
/// of one `build` (ms) plus one middle-of-content `visible_window` (µs).
fn time_build_and_slice(n: usize) -> (u128, u128) {
    let chat = build_large_chat(n);

    let t0 = Instant::now();
    let cache = ViewportCache::build(&chat, 80, 0, 0);
    let build_ms = t0.elapsed().as_millis();

    // Probe the middle of the content so the binary search lands on a non-zero
    // offset rather than a trivial head/tail edge case.
    let mid = cache.total_rows() / 2;
    let t1 = Instant::now();
    let _ = cache.visible_window(mid, 24);
    let slice_us = t1.elapsed().as_micros();

    (build_ms, slice_us)
}

#[test]
fn viewport_build_and_slice_1k_blocks() {
    let (build_ms, slice_us) = time_build_and_slice(1_000);
    // Absolute build-time guard is release-only: in a debug build the O(n)
    // flattening runs ~5-10x slower, so the threshold (tuned for release) is
    // meaningless. The code path still executes here to catch panics; the
    // slice/scaling guards below stay active in every build.
    if !cfg!(debug_assertions) {
        assert!(
            build_ms < 500,
            "1k-block build took {build_ms}ms, expected < 500ms"
        );
    }
    assert!(
        slice_us < 1_000,
        "1k-block visible_window took {slice_us}µs, expected < 1ms"
    );
}

#[test]
fn viewport_build_and_slice_5k_blocks() {
    let (build_ms, slice_us) = time_build_and_slice(5_000);
    if !cfg!(debug_assertions) {
        assert!(
            build_ms < 1_000,
            "5k-block build took {build_ms}ms, expected < 1000ms"
        );
    }
    assert!(
        slice_us < 1_000,
        "5k-block visible_window took {slice_us}µs, expected < 1ms"
    );
}

#[test]
fn viewport_build_and_slice_10k_blocks() {
    let (build_ms, slice_us) = time_build_and_slice(10_000);
    if !cfg!(debug_assertions) {
        assert!(
            build_ms < 3_000,
            "10k-block build took {build_ms}ms, expected < 3000ms"
        );
    }
    assert!(
        slice_us < 1_000,
        "10k-block visible_window took {slice_us}µs, expected < 1ms"
    );
}

/// Measure `iters` frames of pure per-frame work: one `visible_window` lookup
/// plus cloning exactly the visible lines (what `render_body` hands to the
/// `Paragraph` widget). The cache is pre-built, so this isolates the per-frame
/// cost from the amortized build cost. Returns total elapsed nanoseconds.
fn time_per_frame_loop(cache: &ViewportCache, iters: usize) -> u128 {
    let mid = cache.total_rows() / 2;
    let mut sink = 0usize;
    let t0 = Instant::now();
    for _ in 0..iters {
        let (start, end, _top_skip) = cache.visible_window(mid, 24);
        // Clone the visible logical lines exactly as render_body does.
        let visible: Vec<_> = cache.lines()[start..end].to_vec();
        sink = sink.wrapping_add(visible.len());
    }
    let elapsed = t0.elapsed().as_nanos();
    // Prevent the loop body from being optimized away.
    std::hint::black_box(sink);
    elapsed
}

/// The headline A1 assertion: per-frame cost (visible_window + cloning the
/// visible lines) must scale with the viewport height, NOT with the transcript
/// length. We pre-build the cache (the amortized, 3-FPS cost) and then measure
/// 100 frames of pure rendering work on a 1k and a 10k transcript.
///
/// If virtualization were absent, the 10k case would be ~10x slower than the
/// 1k case (it would re-scan the whole transcript each frame). Because
/// `visible_window` is O(log n) and only the visible rows are cloned, the two
/// stay within 2x of each other and each frame is well under 50ms.
#[test]
fn per_frame_cost_bounded_by_visible_h_not_block_count() {
    let chat_1k = build_large_chat(1_000);
    let chat_10k = build_large_chat(10_000);
    let cache_1k = ViewportCache::build(&chat_1k, 80, 0, 0);
    let cache_10k = ViewportCache::build(&chat_10k, 80, 0, 0);

    let total_1k = time_per_frame_loop(&cache_1k, 100);
    let total_10k = time_per_frame_loop(&cache_10k, 100);

    // Absolute bound: a single worst-case frame must be under 50ms even at 10k
    // blocks. A virtualized frame clones only ~24 rows, so this is hugely
    // generous; it exists to fail loudly if the virtualization is removed.
    let per_frame_10k_ns = total_10k / 100;
    assert!(
        per_frame_10k_ns < 50_000_000,
        "per-frame cost at 10k blocks is {}ms, expected < 50ms",
        per_frame_10k_ns / 1_000_000
    );

    // O(visible_h), not O(n): the 10k transcript must be within 2x of the 1k
    // transcript. A 1ms tolerance absorbs measurement noise when the 1k
    // denominator is near zero; if virtualization regressed, 10k would be ~10x.
    let ratio = total_10k as f64 / total_1k.max(1) as f64;
    assert!(
        total_10k <= 2 * total_1k + 1_000_000,
        "per-frame cost scaled {ratio:.2}x from 1k -> 10k blocks (expected ~1x, \
         bound 2x); the viewport is no longer O(visible_h) — likely a \
         virtualization regression"
    );
}
