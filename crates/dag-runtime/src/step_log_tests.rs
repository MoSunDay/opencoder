//! Tests for [`crate::step_log`]: the batching pump's flush policy, the
//! locked `step_output` record shape, per-record caps, split-byte handling,
//! warn-and-drop persistence failures, and the piped-stream tee adapter.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_store::{EventKind, SessionEventRecord};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::step_log::{
    event_record, split_chunks, EventWriter, StepOutputLog, Stream, FLUSH_BYTES, MAX_TEXT_BYTES,
    SSE_KIND,
};

/// Test double recording every appended batch; optionally failing so the
/// warn-and-drop path can be proven.
#[derive(Default)]
pub(crate) struct RecordingWriter {
    batches: Mutex<Vec<Vec<SessionEventRecord>>>,
    calls: AtomicUsize,
    pub(crate) fail: bool,
}

impl RecordingWriter {
    pub(crate) fn batch_count(&self) -> usize {
        self.batches.lock().unwrap().len()
    }

    pub(crate) fn append_calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn rows(&self) -> Vec<SessionEventRecord> {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect()
    }

    pub(crate) fn field(&self, key: &str) -> Vec<String> {
        self.rows()
            .iter()
            .filter_map(|row| row.payload.get(key).and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    pub(crate) fn texts(&self) -> Vec<String> {
        self.field("text")
    }

    pub(crate) fn streams(&self) -> Vec<String> {
        self.field("stream")
    }
}

#[async_trait::async_trait]
impl EventWriter for RecordingWriter {
    async fn append(&self, rows: &[SessionEventRecord]) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("store unavailable");
        }
        self.batches.lock().unwrap().push(rows.to_vec());
        Ok(())
    }
}

/// Spawn a log over the recorder (usable from sibling test modules too).
pub(crate) fn logged(writer: &Arc<RecordingWriter>, run_id: &str, step: &str) -> StepOutputLog {
    StepOutputLog::with_writer(writer.clone(), run_id, step)
}

/// Wait (bounded) for the pump to emit `count` batches.
async fn await_batches(writer: &RecordingWriter, count: usize) {
    for _ in 0..200 {
        if writer.batch_count() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "pump emitted {} batch(es), expected >= {count}",
        writer.batch_count()
    );
}

// --- pure helpers ----------------------------------------------------------

#[test]
fn chunks_split_on_char_boundaries() {
    assert_eq!(split_chunks("", 8), Vec::<String>::new());
    assert_eq!(split_chunks("hello", 8), vec!["hello".to_string()]);
    assert_eq!(split_chunks("abcdef", 2), vec!["ab", "cd", "ef"]);
    // A cap narrower than one char keeps the char whole.
    assert_eq!(split_chunks("中", 1), vec!["中".to_string()]);
    let long = "中".repeat(10);
    for chunk in split_chunks(&long, 4) {
        assert!(chunk.len() <= 4, "chunk {chunk:?} exceeds the cap");
        assert!(chunk.chars().all(|c| c == '中'));
    }
    assert_eq!(split_chunks(&long, 4).concat(), long);
}

#[test]
fn event_record_matches_the_locked_shape() {
    let row = event_record("run-1", "build", Stream::Stderr, "boom", 42);
    assert_eq!(row.session_id, "run-1");
    assert_eq!(row.kind, EventKind::Step);
    assert_eq!(row.sse_kind.as_deref(), Some(SSE_KIND));
    assert_eq!(row.ts, 42);
    assert!(row.seq.is_none());
    assert_eq!(
        row.payload,
        json!({"step":"build","stream":"stderr","text":"boom","at_ms":42})
    );
    assert_eq!(Stream::Stdout.label(), "stdout");
}

// --- pump ------------------------------------------------------------------

#[tokio::test]
async fn close_flushes_the_tail_batch() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    log.push(Stream::Stdout, "hello world");
    log.close().await;
    assert_eq!(writer.batch_count(), 1);
    let row = writer.rows().swap_remove(0);
    assert_eq!(row.payload["step"], "build");
    assert_eq!(row.payload["text"], "hello world");
    assert_eq!(row.payload["at_ms"].as_i64().unwrap(), row.ts);
    // Closing twice must not re-append an (empty) tail.
    log.close().await;
    assert_eq!(writer.batch_count(), 1);
}

#[tokio::test]
async fn oversized_chunk_is_split_into_bounded_records() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    let text = "ab".repeat(MAX_TEXT_BYTES); // 16 KiB
    log.push(Stream::Stdout, &text);
    log.close().await;
    let rows = writer.rows();
    assert!(rows.len() >= 2, "got {} rows", rows.len());
    for row in &rows {
        assert!(row.payload["text"].as_str().unwrap().len() <= MAX_TEXT_BYTES);
        assert_eq!(row.payload["at_ms"].as_i64().unwrap(), row.ts);
    }
    assert_eq!(writer.texts().concat(), text);
}

#[tokio::test]
async fn byte_threshold_flushes_without_close() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    log.push(Stream::Stdout, &"x".repeat(FLUSH_BYTES + 1));
    await_batches(&writer, 1).await;
    assert_eq!(
        writer.texts().iter().map(String::len).sum::<usize>(),
        FLUSH_BYTES + 1
    );
    log.close().await;
}

#[tokio::test]
async fn window_flushes_a_small_batch_without_close() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    log.push(Stream::Stdout, "tick");
    await_batches(&writer, 1).await;
    assert_eq!(writer.texts(), vec!["tick".to_string()]);
    log.close().await;
}

#[tokio::test]
async fn append_failure_is_warn_and_drop() {
    let writer = Arc::new(RecordingWriter {
        fail: true,
        ..Default::default()
    });
    let log = logged(&writer, "run-1", "build");
    log.push(Stream::Stdout, "doomed");
    log.close().await; // must neither panic nor propagate
    assert_eq!(writer.append_calls(), 1, "the batch was attempted");
    assert_eq!(writer.batch_count(), 0);
}

#[tokio::test]
async fn split_utf8_and_invalid_bytes_survive_pushes() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    // A multi-byte char straddling two writes must not turn into U+FFFD.
    let encoded = "中文".as_bytes();
    log.push_bytes(Stream::Stdout, &encoded[..4]);
    log.push_bytes(Stream::Stdout, &encoded[4..]);
    // Invalid UTF-8 is mirrored lossily (replacement chars), not dropped.
    log.push_bytes(Stream::Stderr, &[0x6f, 0x6b, 0xff]);
    log.close().await;
    assert_eq!(
        writer.texts(),
        vec!["中文".to_string(), "ok\u{fffd}".to_string()]
    );
    assert_eq!(
        writer.streams(),
        vec!["stdout".to_string(), "stderr".to_string()]
    );
}

#[tokio::test]
async fn tee_reader_mirrors_piped_bytes() {
    let writer = Arc::new(RecordingWriter::default());
    let log = logged(&writer, "run-1", "build");
    let piped: &[u8] = b"piped output";
    let mut tee = log.tee_reader(Stream::Stderr, piped);
    let mut sink = Vec::new();
    tee.read_to_end(&mut sink).await.unwrap();
    assert_eq!(sink, b"piped output");
    log.close().await;
    assert_eq!(writer.texts(), vec!["piped output".to_string()]);
    assert_eq!(writer.streams(), vec!["stderr".to_string()]);
}
