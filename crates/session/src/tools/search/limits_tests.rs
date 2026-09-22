use super::*;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};

fn context(root: &Path) -> ToolContext {
    ToolContext {
        extra_env: vec![],
        session_id: "limits".into(),
        message_id: "limits".into(),
        agent: "explore".into(),
        working_dir: root.into(),
        max_output: 4096,
        proxy: None,
        tools_path: None,
    }
}

#[tokio::test]
async fn sparse_binary_does_not_hide_neighboring_text_match() {
    let root = tempfile::tempdir().unwrap();
    // A sparse file costs no resident allocation; the pre-fix searcher read
    // the whole NUL-filled line into its heap despite advertising binary skips.
    std::fs::File::create(root.path().join("image.bin"))
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    std::fs::write(root.path().join("text.txt"), "needle\n").unwrap();
    let out = SearchTool
        .execute(
            serde_json::json!({"pattern":"needle|\\x00"}),
            &context(root.path()),
        )
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("text.txt:1: needle"));
    assert!(!out.content.contains("image.bin"));
}

#[tokio::test]
async fn oversized_text_line_is_an_explicit_error_not_false_no_matches() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("large.txt"),
        vec![b'x'; bounded::LINE_BYTES * 2],
    )
    .unwrap();
    let out = SearchTool
        .execute(
            serde_json::json!({"pattern":"absent","path":"large.txt"}),
            &context(root.path()),
        )
        .await
        .unwrap();
    assert!(out.is_error, "{}", out.content);
    assert!(
        out.content.contains("large.txt") && out.content.contains("alloc"),
        "{}",
        out.content
    );
}

#[tokio::test]
async fn missing_search_path_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let out = SearchTool
        .execute(
            serde_json::json!({"pattern":"needle","path":"missing"}),
            &context(root.path()),
        )
        .await
        .unwrap();
    assert!(out.is_error, "{}", out.content);
}

struct Endless {
    reads: Arc<AtomicUsize>,
    byte: u8,
}
impl Read for Endless {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        bytes.fill(self.byte);
        Ok(bytes.len())
    }
}

#[test]
fn binary_detection_stops_an_endless_nul_stream_after_one_chunk() {
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = Endless {
        reads: reads.clone(),
        byte: 0,
    };
    let matcher = RegexMatcherBuilder::new().build("absent").unwrap();
    bounded::searcher()
        .search_reader(
            &matcher,
            reader,
            grep_searcher::sinks::Bytes(|_, _| Ok(true)),
        )
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_interrupts_a_nonmatching_reader_before_another_read() {
    let cancel = CancellationToken::new();
    let reads = Arc::new(AtomicUsize::new(0));
    let mut reader = bounded::CancelReader {
        inner: Endless {
            reads: reads.clone(),
            byte: b'x',
        },
        cancel: &cancel,
    };
    reader.read_exact(&mut [0; 8]).unwrap();
    cancel.cancel();
    let error = reader.read(&mut [0; 8]).unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn matching_long_lines_stop_at_the_collector_byte_budget() {
    let cancel = CancellationToken::new();
    let mut collector = Collector {
        results: vec![],
        rel: "large.txt".into(),
        max: MAX_MATCHES,
        bytes: 0,
        truncated: false,
        cancel,
    };
    let text = format!("{}\n", "needle中文".repeat(10000));
    let matcher = RegexMatcherBuilder::new().build("needle").unwrap();
    bounded::searcher()
        .search_reader(&matcher, text.as_bytes(), &mut collector)
        .unwrap();
    assert!(collector.truncated);
    assert!(collector.results.iter().map(String::len).sum::<usize>() <= bounded::OUTPUT_BYTES);
    assert_eq!(collector.results.len(), 1);
}

#[tokio::test]
async fn dropped_async_search_stops_its_started_blocking_reader() {
    struct PausedRead {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Read for PausedRead {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
                self.release
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                bytes.fill(b'\n');
                Ok(bytes.len())
            } else {
                Ok(0)
            }
        }
    }
    let (started, waiting) = tokio::sync::oneshot::channel();
    let (release, hold) = std::sync::mpsc::channel();
    let (finished, result) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(bounded::run(move |cancel| {
        let matcher = RegexMatcherBuilder::new().build("absent").unwrap();
        let reader = bounded::CancelReader {
            inner: PausedRead {
                started: Some(started),
                release: hold,
            },
            cancel: &cancel,
        };
        let outcome = bounded::searcher().search_reader(
            &matcher,
            reader,
            grep_searcher::sinks::Bytes(|_, _| Ok(true)),
        );
        let _ = finished.send(outcome.map_err(|error| error.to_string()));
        ToolOutput::ok("")
    }));
    waiting.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), result)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
}
