//! Lifecycle serialization regression tests. Drain starts and idle-only
//! setting changes must contend on the same per-session mutex, so whichever
//! operation wins is complete before the other observes `draining`.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::response::IntoResponse;
use opencoder_core::Config;
use opencoder_store::{Delivery, SessionMeta, Store, TursoStore};
use opencoder_web::handle::SessionHandle;

mod support;

use support::hanging_stream::HangingStream;

async fn state(sid: &str) -> (Arc<opencoder_web::AppState>, Arc<SessionHandle>) {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
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
            config_home: None,
            client_override: None,
            brain: opencoder_web::api_brain::mock_brain(store.clone()),
            store,
            workdir: std::env::temp_dir(),
            handles,

            nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
            controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
            team: opencoder_web::team_state::mock(),
            project: opencoder_web::ProjectService::new(),
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

/// Poll until the session handle's agent override matches `want`. The
/// override write happens inside the switch's lifecycle critical section, so
/// observing it proves the switch holds the mutex mid-critical-section.
async fn wait_agent_override(handle: &SessionHandle, want: &str) {
    for _ in 0..200 {
        if handle.overrides.lock().await.agent.as_deref() == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("agent override never reached {want:?}");
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
    // Release the guard so the switch can enter its critical section, then
    // wait for a write that only happens while it holds the lifecycle mutex.
    // Spawning the drain only after that observation makes the outcome follow
    // from mutual exclusion, not from task-poll timing: the switch task has
    // a store roundtrip before its lock acquire while the drain path reaches
    // the lock directly, so a bare yield cannot order them.
    drop(lifecycle);
    wait_agent_override(&handle, "plan").await;
    let drain_state = state.clone();
    let drain = tokio::spawn(async move { start_hanging_drain(drain_state, sid).await });

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
