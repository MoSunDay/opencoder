use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

#[derive(Clone, Default)]
pub(super) struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    pub(super) fn reader(&self, reader: Box<dyn AsyncRead + Unpin + Send>) -> Reader {
        Reader {
            reader,
            capture: self.clone(),
        }
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

pub(super) struct Reader {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    capture: Capture,
}

impl AsyncRead for Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.reader).poll_read(context, buffer);
        if matches!(result, Poll::Ready(Ok(()))) {
            let mut bytes = self.capture.0.lock().unwrap();
            let remaining = (64 * 1024usize).saturating_sub(bytes.len());
            let read = &buffer.filled()[before..];
            bytes.extend_from_slice(&read[..remaining.min(read.len())]);
        }
        result
    }
}

#[derive(Debug)]
pub struct ProcessFailure {
    pub output: String,
    error: anyhow::Error,
}

impl ProcessFailure {
    pub(super) fn new(error: anyhow::Error, output: String) -> Self {
        Self { error, output }
    }
}

impl std::fmt::Display for ProcessFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.error)
    }
}
impl std::error::Error for ProcessFailure {}
