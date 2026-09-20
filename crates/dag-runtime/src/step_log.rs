//! Incremental step output mirrored into the NODE store as `step_output`
//! events on the run's session (`session_id = run_id`).
//!
//! Record shape (LOCKED wire contract — node-side consumers filter run
//! sessions by `payload.step` and select step output by `sse_kind`):
//!
//! ```text
//! SessionEventRecord {
//!   session_id: <run_id>, kind: EventKind::Step, sse_kind: Some("step_output"),
//!   payload: {"step":<step>,"stream":"stdout"|"stderr","text":<chunk>,"at_ms":<now_ms>},
//!   ts: <at_ms>, seq: None,
//! }
//! ```
//!
//! Flushing: one append per 300 ms window or per accumulated 4 KiB, with a
//! single `text` capped at 8 KiB (split on char boundaries). The tail batch
//! is always flushed before the step reports its terminal outcome
//! ([`StepOutputLog::close`]) — success, timeout, cancel, and error alike.
//!
//! Mirroring is best-effort by design: a failing store only logs a warning
//! and drops the batch (the same warn-and-drop posture as
//! [`crate::dag_events`]) — step output never fails a step.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_store::{EventKind, SessionEventRecord, Store};
use serde_json::json;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::mpsc;
use tracing::warn;

/// Flush at least this often while output keeps arriving.
pub const FLUSH_INTERVAL_MS: u64 = 300;
/// ... or as soon as this many bytes are buffered.
pub const FLUSH_BYTES: usize = 4 * 1024;
/// Hard cap for one record's `text` (longer chunks are split).
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
/// `sse_kind` of every mirrored row.
pub const SSE_KIND: &str = "step_output";

/// Which captured stream a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    /// Wire value of `payload.stream`.
    pub fn label(self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        }
    }

    fn index(self) -> usize {
        match self {
            Stream::Stdout => 0,
            Stream::Stderr => 1,
        }
    }
}

/// Persistence seam so tests can record (or fail) appends without a store.
#[async_trait::async_trait]
pub trait EventWriter: Send + Sync {
    async fn append(&self, rows: &[SessionEventRecord]) -> Result<()>;
}

/// [`EventWriter`] over the node store's batch append.
pub struct StoreWriter(pub Arc<dyn Store>);

#[async_trait::async_trait]
impl EventWriter for StoreWriter {
    async fn append(&self, rows: &[SessionEventRecord]) -> Result<()> {
        self.0.append_events(rows).await.map(|_| ())
    }
}

/// Producer handle for one step's mirrored output. Cheap to clone: clones
/// share the pump task, and the pump exits (flushing its tail) once every
/// clone is dropped or [`StepOutputLog::close`] runs.
#[derive(Clone)]
pub struct StepOutputLog {
    tx: mpsc::UnboundedSender<Cmd>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

enum Cmd {
    Push(Stream, Vec<u8>),
    Stop,
}

impl StepOutputLog {
    /// Mirror one step's output into `store`. Must be called inside a tokio
    /// runtime (it spawns the batching pump).
    pub fn new(store: Arc<dyn Store>, run_id: &str, step: &str) -> Self {
        Self::with_writer(Arc::new(StoreWriter(store)), run_id, step)
    }

    /// [`StepOutputLog::new`] over an explicit writer (test seam).
    pub fn with_writer(writer: Arc<dyn EventWriter>, run_id: &str, step: &str) -> Self {
        Self::with_identity(writer, run_id, step, None)
    }

    pub fn for_instance(
        store: Arc<dyn Store>,
        run_id: &str,
        step: &str,
        index: Option<usize>,
    ) -> Self {
        Self::with_identity(Arc::new(StoreWriter(store)), run_id, step, index)
    }

    fn with_identity(
        writer: Arc<dyn EventWriter>,
        run_id: &str,
        step: &str,
        index: Option<usize>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let join = tokio::spawn(pump(
            writer,
            run_id.to_string(),
            step.to_string(),
            index,
            rx,
        ));
        Self {
            tx,
            join: Arc::new(Mutex::new(Some(join))),
        }
    }

    /// Mirror a text chunk (see [`StepOutputLog::push_bytes`]).
    pub fn push(&self, stream: Stream, text: &str) {
        self.push_bytes(stream, text.as_bytes());
    }

    /// Mirror raw bytes. Never blocks and never fails: a stopped pump (or a
    /// dropped handle) simply drops the chunk.
    pub fn push_bytes(&self, stream: Stream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = self.tx.send(Cmd::Push(stream, bytes.to_vec()));
    }

    /// Wrap a piped stream so every byte read from it is mirrored — the
    /// `runc` child's stdout/stderr are drained by a bounded collector, and
    /// this adapter makes that path incremental without touching it.
    pub fn tee_reader<R: AsyncRead + Unpin>(&self, stream: Stream, reader: R) -> TeeReader<R> {
        TeeReader {
            reader,
            log: Some(self.clone()),
            stream,
        }
    }

    /// Stop the pump and WAIT for the tail flush. Callers invoke this before
    /// a step's terminal outcome is published so no output is lost on
    /// success, timeout, cancellation, or error. Idempotent.
    pub async fn close(&self) {
        let _ = self.tx.send(Cmd::Stop);
        let join = self.join.lock().ok().and_then(|mut guard| guard.take());
        if let Some(join) = join {
            if let Err(e) = join.await {
                warn!(error = %e, "step output pump panicked");
            }
        }
    }
}

/// [`AsyncRead`] adapter mirroring every read chunk into a [`StepOutputLog`].
pub struct TeeReader<R> {
    reader: R,
    log: Option<StepOutputLog>,
    stream: Stream,
}

impl<R: AsyncRead + Unpin> AsyncRead for TeeReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.reader).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let filled = buf.filled();
                if filled.len() > before {
                    if let Some(log) = &this.log {
                        log.push_bytes(this.stream, &filled[before..]);
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

/// Batching pump: decode, buffer, and append on the interval/byte policy.
async fn pump(
    writer: Arc<dyn EventWriter>,
    run_id: String,
    step: String,
    index: Option<usize>,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
) {
    let mut pending = Pending::default();
    let mut tick = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Push(stream, bytes)) => {
                    pending.push(stream, &bytes);
                    if pending.bytes >= FLUSH_BYTES {
                        flush(&writer, &run_id, &step, index, &mut pending).await;
                    }
                }
                // Stop (or every producer dropped): leave the loop so the
                // tail below is the last thing this step writes.
                Some(Cmd::Stop) | None => break,
            },
            _ = tick.tick() => {
                if pending.due() {
                    flush(&writer, &run_id, &step, index, &mut pending).await;
                }
            }
        }
    }
    pending.flush_partial();
    flush(&writer, &run_id, &step, index, &mut pending).await;
}

/// Buffered output waiting for its batch: decoded segments in arrival order
/// plus the per-stream incomplete-UTF-8 tail (a multi-byte char may straddle
/// two guest writes).
#[derive(Default)]
struct Pending {
    segments: Vec<(Stream, String)>,
    partial: [Vec<u8>; 2],
    bytes: usize,
    queued_at_ms: Option<i64>,
}

impl Pending {
    fn push(&mut self, stream: Stream, bytes: &[u8]) {
        let partial = &mut self.partial[stream.index()];
        partial.extend_from_slice(bytes);
        let complete = match std::str::from_utf8(partial) {
            Ok(_) => partial.len(),
            // An unexpected end means "character continues in the next
            // write", so retain that suffix. Anything else is invalid bytes
            // we display lossily and can consume through the invalid byte.
            Err(e) => e.valid_up_to().saturating_add(e.error_len().unwrap_or(0)),
        };
        if complete == 0 {
            return;
        }
        let drained: Vec<u8> = partial.drain(..complete).collect();
        let text = String::from_utf8_lossy(&drained).into_owned();
        if text.is_empty() {
            return;
        }
        self.bytes += text.len();
        if let Some((last_stream, last_text)) = self.segments.last_mut() {
            if *last_stream == stream {
                last_text.push_str(&text);
            } else {
                self.segments.push((stream, text));
            }
        } else {
            self.segments.push((stream, text));
        }
        self.queued_at_ms.get_or_insert_with(now_ms);
    }

    /// Interval flush condition: bytes over the cap, or a batch that has
    /// been waiting for a full window.
    fn due(&self) -> bool {
        self.bytes >= FLUSH_BYTES
            || self
                .queued_at_ms
                .is_some_and(|at| now_ms().saturating_sub(at) >= FLUSH_INTERVAL_MS as i64)
    }

    /// Emit the incomplete tail as replacement chars rather than drop it.
    fn flush_partial(&mut self) {
        for (index, stream) in [Stream::Stdout, Stream::Stderr].iter().enumerate() {
            let bytes = std::mem::take(&mut self.partial[index]);
            if bytes.is_empty() {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            self.bytes += text.len();
            self.segments.push((*stream, text));
        }
    }
}

/// One append of everything buffered; failures are warn-and-drop.
async fn flush(
    writer: &Arc<dyn EventWriter>,
    run_id: &str,
    step: &str,
    index: Option<usize>,
    pending: &mut Pending,
) {
    pending.queued_at_ms = None;
    pending.bytes = 0;
    if pending.segments.is_empty() {
        return;
    }
    let at_ms = now_ms();
    let mut rows: Vec<SessionEventRecord> = Vec::new();
    for (stream, text) in std::mem::take(&mut pending.segments) {
        for chunk in split_chunks(&text, MAX_TEXT_BYTES) {
            let mut row = event_record(run_id, step, stream, &chunk, at_ms);
            if let Some(index) = index {
                row.payload["index"] = json!(index);
            }
            rows.push(row);
        }
    }
    if let Err(e) = writer.append(&rows).await {
        warn!(
            %run_id, %step, events = rows.len(), error = %e,
            "step_output persistence failed (batch dropped)"
        );
    }
}

/// The wire record for one mirrored chunk.
pub fn event_record(
    run_id: &str,
    step: &str,
    stream: Stream,
    text: &str,
    at_ms: i64,
) -> SessionEventRecord {
    SessionEventRecord {
        session_id: run_id.to_string(),
        kind: EventKind::Step,
        payload: json!({
            "step": step,
            "stream": stream.label(),
            "text": text,
            "at_ms": at_ms,
        }),
        ts: at_ms,
        seq: None,
        sse_kind: Some(SSE_KIND.into()),
    }
}

/// Split `text` into pieces of at most `max_bytes`, never cutting a char.
pub fn split_chunks(text: &str, max_bytes: usize) -> Vec<String> {
    let max = max_bytes.max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max).min(text.len());
        if end < text.len() {
            while end > start && !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == start {
                // A cap narrower than one char: keep that char whole.
                let wide = text[start..]
                    .chars()
                    .next()
                    .map_or(1, |c| c.len_utf8())
                    .min(text.len() - start);
                end = start + wide;
            }
        }
        chunks.push(text[start..end].to_string());
        start = end;
    }
    chunks
}
