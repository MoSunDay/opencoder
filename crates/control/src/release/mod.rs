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
use futures::StreamExt;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
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
    let stream = futures::stream::unfold(
        (body.into_data_stream(), guard),
        |(mut body, guard)| async move { body.next().await.map(|chunk| (chunk, (body, guard))) },
    );
    Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

pub async fn retire(State(state): State<Arc<crate::AppState>>) -> Json<serde_json::Value> {
    state.lifecycle.retire();
    Json(serde_json::json!({"retiring":true}))
}
