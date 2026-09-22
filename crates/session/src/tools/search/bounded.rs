//! Bound the search engine itself, before result rendering can truncate output.
use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink};
use opencoder_core::ToolOutput;
use std::{fs::File, io, io::Read, path::Path};
use tokio_util::sync::CancellationToken;

pub(super) const LINE_BYTES: usize = 1024 * 1024;
pub(super) const OUTPUT_BYTES: usize = 64 * 1024;

pub(super) async fn run<F>(job: F) -> ToolOutput
where
    F: FnOnce(CancellationToken) -> ToolOutput + Send + 'static,
{
    let cancel = CancellationToken::new();
    // A tool timeout drops this future, but cannot abort spawn_blocking.
    let _guard = cancel.clone().drop_guard();
    tokio::task::spawn_blocking(move || job(cancel))
        .await
        .unwrap_or_else(|error| ToolOutput::err(format!("search task failed: {error}")))
}

pub(super) fn searcher() -> Searcher {
    SearcherBuilder::new()
        .line_number(true)
        .binary_detection(BinaryDetection::quit(0))
        .heap_limit(Some(LINE_BYTES))
        .build()
}

pub(super) fn search_file<S: Sink<Error = io::Error>>(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    sink: S,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("search requires a regular file"));
    }
    searcher.search_reader(
        matcher,
        CancelReader {
            inner: file,
            cancel,
        },
        sink,
    )
}

/// Checking only matches misses huge nonmatching files. Check each bounded read
/// as well so a dropped async call releases its blocking task and line buffer.
pub(super) struct CancelReader<'a, R> {
    pub(super) inner: R,
    pub(super) cancel: &'a CancellationToken,
}

impl<R: Read> Read for CancelReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            // Interrupted is automatically retried by some readers.
            return Err(io::Error::other("search cancelled"));
        }
        let len = bytes.len().min(64 * 1024);
        self.inner.read(&mut bytes[..len])
    }
}
