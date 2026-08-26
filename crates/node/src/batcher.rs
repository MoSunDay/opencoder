//! Event batching for the node→server uplink.
//!
//! Pure accumulation logic with no IO: events accumulate until either the
//! count cap or a time window expires. Flush decisions and draining are kept
//! separate so the caller controls transport — the batcher never touches the
//! network. The window uses `tokio::time::Instant`, so `tokio::time::pause`
//! tests the threshold deterministically.

use opencoder_core::message::now_ms;
use opencoder_core::node_protocol::NodeEventIn;

/// Upload as soon as this many events are buffered.
pub const MAX_EVENTS: usize = 32;
/// ... or once this long passed since the first buffered event.
pub const WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug)]
pub struct Batcher {
    buf: Vec<NodeEventIn>,
    /// Set on first push after an empty state (`None` = window closed).
    opened: Option<tokio::time::Instant>,
}

impl Default for Batcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Batcher {
    pub fn new() -> Self {
        Batcher {
            buf: Vec::new(),
            opened: None,
        }
    }

    /// Buffer one event; opens the flush window on the first push.
    pub fn push(&mut self, ev: NodeEventIn) {
        if self.opened.is_none() {
            self.opened = Some(tokio::time::Instant::now());
        }
        self.buf.push(ev);
    }

    /// A flush is due when the count cap is reached, or the open window aged
    /// past [`WINDOW`]. Never true while empty.
    pub fn should_flush(&self) -> bool {
        if self.buf.is_empty() {
            return false;
        }
        if self.buf.len() >= MAX_EVENTS {
            return true;
        }
        self.opened.is_some_and(|opened| opened.elapsed() >= WINDOW)
    }

    /// Drain every buffered event in order and close the window so the next
    /// push starts a fresh one.
    pub fn take(&mut self) -> Vec<NodeEventIn> {
        self.opened = None;
        std::mem::take(&mut self.buf)
    }
}

/// Build one wire event from its parts (kept next to the batcher so uploads
/// always stamp a consistent emitter clock).
pub fn wire_event(sse_kind: &str, payload: serde_json::Value) -> NodeEventIn {
    NodeEventIn {
        sse_kind: sse_kind.to_string(),
        payload,
        ts: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(n: usize) -> NodeEventIn {
        wire_event("text_delta", serde_json::json!({ "text": n }))
    }

    #[test]
    fn max_events_flushes_immediately() {
        let mut b = Batcher::new();
        for n in 0..MAX_EVENTS {
            b.push(ev(n));
        }
        assert!(b.should_flush(), "exactly {MAX_EVENTS} events must flush");
    }

    #[test]
    fn below_max_with_closed_window_holds() {
        let mut b = Batcher::new();
        for n in 0..MAX_EVENTS - 1 {
            b.push(ev(n));
        }
        assert!(
            !b.should_flush(),
            "31 fresh events must wait for the window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn aged_window_flushes_before_count_cap() {
        let mut b = Batcher::new();
        b.push(ev(0));
        assert!(!b.should_flush(), "one fresh event must wait");
        tokio::time::advance(WINDOW).await;
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        // 301 ms elapsed since the single push → time wins over count.
        b.push(ev(1));
        assert!(
            b.should_flush(),
            "an open window past {WINDOW:?} must flush even with few events"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn take_empties_buffer_and_reopens_window_later() {
        let mut b = Batcher::new();
        b.push(ev(0));
        tokio::time::advance(WINDOW).await;
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert!(b.should_flush());
        let drained = b.take();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].sse_kind, "text_delta");
        assert!(
            !b.should_flush(),
            "a drained batcher must behave like a fresh one"
        );
        // The window stays closed until the NEXT push (the extra advance
        // proves the leftover aged window was closed by `take`).
        tokio::time::advance(WINDOW).await;
        assert!(!b.should_flush());
        b.push(ev(2));
        assert!(!b.should_flush(), "push reopens a NEW window");
    }
}
