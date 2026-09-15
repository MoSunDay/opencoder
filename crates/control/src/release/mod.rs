pub(crate) mod outbox;
mod proxy;
pub mod resources;
pub use proxy::{forward_resources, status};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
    Json,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Default)]
pub struct Lifecycle {
    pub outbox_started: AtomicBool,
    pub retiring: AtomicBool,
    pub requests: AtomicUsize,
    pub changed: tokio::sync::Notify,
    pub credential: OnceLock<String>,
    pub platform: OnceLock<opencoder_core::fleet::release::PlatformConfig>,
}

impl Lifecycle {
    pub fn retire(&self) {
        self.retiring.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }
    pub async fn retired(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.retiring.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }
    pub async fn drained(&self) {
        while self.requests.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

struct RequestGuard(Arc<Lifecycle>);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.requests.fetch_sub(1, Ordering::SeqCst);
    }
}

struct TrackedBody {
    body: axum::body::Body,
    _guard: RequestGuard,
}

impl http_body::Body for TrackedBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.body).poll_frame(context)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

/// Count complete response bodies, including SSE and artifact downloads.
/// Retirement has no deadline that can cut off an ordinary accepted request.
pub async fn track(
    State(state): State<Arc<crate::AppState>>,
    request: Request,
    next: Next,
) -> Response {
    state.lifecycle.requests.fetch_add(1, Ordering::SeqCst);
    let guard = RequestGuard(state.lifecycle.clone());
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        axum::body::Body::new(TrackedBody {
            body,
            _guard: guard,
        }),
    )
}

pub async fn retire(State(state): State<Arc<crate::AppState>>) -> Json<serde_json::Value> {
    state.lifecycle.retire();
    Json(serde_json::json!({"retiring":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body::Body as _;

    struct Trailers(Option<axum::http::HeaderMap>);
    impl http_body::Body for Trailers {
        type Data = axum::body::Bytes;
        type Error = axum::Error;
        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(
                self.0
                    .take()
                    .map(|headers| Ok(http_body::Frame::trailers(headers))),
            )
        }
    }

    #[tokio::test]
    async fn tracking_preserves_response_length_trailers_and_body_lifetime() {
        let lifecycle = Arc::new(Lifecycle::default());
        lifecycle.requests.store(1, Ordering::SeqCst);
        let body = TrackedBody {
            body: axum::body::Body::from("complete response"),
            _guard: RequestGuard(lifecycle.clone()),
        };
        assert_eq!(body.size_hint().exact(), Some(17));
        lifecycle.retire();
        assert_eq!(lifecycle.requests.load(Ordering::SeqCst), 1);
        drop(body);
        assert_eq!(lifecycle.requests.load(Ordering::SeqCst), 0);
        let mut trailers = axum::http::HeaderMap::new();
        trailers.insert("x-result", "durable".parse().unwrap());
        lifecycle.requests.store(1, Ordering::SeqCst);
        let mut body = TrackedBody {
            body: axum::body::Body::new(Trailers(Some(trailers))),
            _guard: RequestGuard(lifecycle.clone()),
        };
        let frame = futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.into_trailers().unwrap()["x-result"], "durable");
        assert_eq!(lifecycle.requests.load(Ordering::SeqCst), 1);
        drop(body);
        lifecycle.drained().await;
        assert_eq!(lifecycle.requests.load(Ordering::SeqCst), 0);
    }
}
