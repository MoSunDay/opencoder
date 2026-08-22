//! Lifecycle serialization regression tests. Drain starts and idle-only
//! setting changes must contend on the same per-session mutex, so whichever
//! operation wins is complete before the other observes `draining`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::response::IntoResponse;
use opencoder_core::Config;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store};
use opencoder_web::handle::SessionHandle;
use tokio::sync::mpsc;

struct HangingStream {
    calls: AtomicUsize,
}

impl HangingStream {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ChatStream for HangingStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(tx);
        });
        Ok(rx)
    }
}

async fn state(sid: &str) -> (Arc<opencoder_web::AppState>, Arc<SessionHandle>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            agent: Some("act".into()),
            model: Some("old/model".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = SessionHandle::new();
    let handles = opencoder_web::handle::new_handle_map();
    handles.lock().await.insert(sid.into(), handle.clone());
    (
        Arc::new(opencoder_web::AppState {
            client_override: None,
            store,
            workdir: std::env::temp_dir(),
            handles,
        }),
        handle,
    )
}

async fn start_hanging_drain(state: Arc<opencoder_web::AppState>, sid: &str) -> i64 {
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        "work".into(),
        vec![],
        Delivery::Steer,
        Arc::new(HangingStream::new()),
        std::env::temp_dir(),
        Config {
            model: "test/model".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

async fn status(response: axum::response::Response) -> axum::http::StatusCode {
    response.status()
}

#[tokio::test]
async fn drain_start_wins_mutex_then_agent_and_model_are_refused() {
    let sid = "drain-first";
    let (state, handle) = state(sid).await;
    let lifecycle = handle.lifecycle.lock().await;

    let drain_state = state.clone();
    let drain = tokio::spawn(async move { start_hanging_drain(drain_state, sid).await });
    tokio::task::yield_now().await;

    let agent_state = state.clone();
    let agent = tokio::spawn(async move {
        opencoder_web::api::post_agent(
            axum::extract::State(agent_state),
            axum::extract::Path(sid.to_string()),
            axum::Json(opencoder_web::api::SwitchBody {
                value: "plan".into(),
            }),
        )
        .await
        .into_response()
    });
    let model_state = state.clone();
    let model = tokio::spawn(async move {
        opencoder_web::api::post_model(
            axum::extract::State(model_state),
            axum::extract::Path(sid.to_string()),
            axum::Json(opencoder_web::api::ModelBody {
                value: "new/model".into(),
                persist_default: false,
            }),
        )
        .await
        .into_response()
    });
    drop(lifecycle);

    assert!(drain.await.unwrap() > 0);
    assert_eq!(
        status(agent.await.unwrap()).await,
        axum::http::StatusCode::CONFLICT
    );
    assert_eq!(
        status(model.await.unwrap()).await,
        axum::http::StatusCode::CONFLICT
    );
    let meta = state.store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
    assert_eq!(meta.model.as_deref(), Some("old/model"));
    let overrides = handle.overrides.lock().await;
    assert!(overrides.agent.is_none());
    assert!(overrides.model.is_none());
    drop(overrides);
    handle.cancel.lock().await.cancel();
}

#[tokio::test]
async fn agent_switch_wins_mutex_and_finishes_before_drain_start() {
    let sid = "switch-first";
    let (state, handle) = state(sid).await;
    let lifecycle = handle.lifecycle.lock().await;

    let agent_state = state.clone();
    let agent = tokio::spawn(async move {
        opencoder_web::api::post_agent(
            axum::extract::State(agent_state),
            axum::extract::Path(sid.to_string()),
            axum::Json(opencoder_web::api::SwitchBody {
                value: "plan".into(),
            }),
        )
        .await
        .into_response()
    });
    tokio::task::yield_now().await;
    let drain_state = state.clone();
    let drain = tokio::spawn(async move { start_hanging_drain(drain_state, sid).await });
    drop(lifecycle);

    assert_eq!(
        status(agent.await.unwrap()).await,
        axum::http::StatusCode::OK
    );
    assert!(drain.await.unwrap() > 0);
    assert!(handle.draining.load(Ordering::SeqCst));
    let meta = state.store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"));
    assert_eq!(handle.overrides.lock().await.agent.as_deref(), Some("plan"));
    handle.cancel.lock().await.cancel();
}
