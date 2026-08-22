//! End-to-end HTTP contract for the running mode gate against the real Axum
//! router and drain lifecycle, with a deliberately hanging LLM stream.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{Delivery, LibsqlStore, SessionMeta, Store};
use tokio::sync::mpsc;
use tower::ServiceExt;

struct HangingStream {
    calls: AtomicUsize,
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

async fn request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn every_manual_mode_path_is_side_effect_free_while_running() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "running-mode";
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            agent: Some("act".into()),
            model: Some("test/model".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir: tmp.path().to_path_buf(),
        handles: opencoder_web::handle::new_handle_map(),
        client_override: Some(llm.clone()),
    });
    let app = opencoder_web::build_app(state.clone(), None, false);

    let started = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"long work","delivery":"steer"}"#,
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    for _ in 0..100 {
        if llm.calls.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        llm.calls.load(Ordering::SeqCst) > 0,
        "drain never reached LLM"
    );

    for (uri, body) in [
        (format!("/api/sessions/{sid}/agent"), r#"{"value":"plan"}"#),
        (
            format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"/plan review","delivery":"queue","skill":"reviewer"}"#,
        ),
        (
            format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"ordinary text","agent":"plan","delivery":"queue"}"#,
        ),
        (format!("/api/sessions/{sid}/handoff"), r#"{"extra":"now"}"#),
    ] {
        let response = request(&app, "POST", &uri, body).await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{uri} accepted");
    }

    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
    assert!(
        meta.skill.is_none(),
        "rejected mode prompt persisted its skill"
    );
    for delivery in [Delivery::Steer, Delivery::Queue] {
        let pending = store.pending_inputs(sid, delivery).await.unwrap();
        assert!(
            pending
                .iter()
                .all(|input| !input.prompt.contains("/plan") && input.prompt != "ordinary text"),
            "rejected mode request was admitted: {pending:?}"
        );
    }
    let messages = store.load_messages(sid).await.unwrap();
    assert!(messages
        .iter()
        .all(|message| !message.text().contains("/plan")));

    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    for _ in 0..100 {
        if !handle.draining.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let idle_switch = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/agent"),
        r#"{"value":"plan"}"#,
    )
    .await;
    assert_eq!(idle_switch.status(), StatusCode::OK);
    assert_eq!(
        store
            .get_session(sid)
            .await
            .unwrap()
            .unwrap()
            .agent
            .as_deref(),
        Some("plan")
    );
}
