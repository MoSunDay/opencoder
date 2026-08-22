//! End-to-end HTTP contract for the running mode gate against the real Axum
//! router and drain lifecycle, with a deliberately hanging LLM stream.
//!
//! Contract split:
//! - Admission-time mode changes (the `agent` field, POST /agent, POST
//!   /handoff) stay 409 while draining — they rewrite session config.
//! - Textual mode commands (/plan, /act, /act_clear_context) are ADMITTED
//!   while running (queue or steer) and applied by the runner at the next
//!   idle/turn boundary, which structurally has no turn in flight.

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

/// Start a drain that parks in a hanging LLM call; returns once the LLM has
/// been reached (the session is deterministically "running").
async fn start_running_drain(
    app: &axum::Router,
    store: &Arc<dyn Store>,
    sid: &str,
    llm: &HangingStream,
) {
    let started = request(
        app,
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
    let _ = store;
}

/// Poll until the session's drain is idle (`draining` false).
async fn wait_idle(state: &Arc<opencoder_web::AppState>, sid: &str) {
    for _ in 0..400 {
        let idle = state
            .handles
            .lock()
            .await
            .get(sid)
            .map(|h| !h.draining.load(Ordering::SeqCst))
            .unwrap_or(true);
        if idle {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("drain for {sid} never went idle");
}

/// Poll until a persisted event of the given SSE kind appears.
async fn wait_for_event_kind(store: &Arc<dyn Store>, sid: &str, kind: &str) {
    for _ in 0..200 {
        let events = store.events_after(sid, 0).await.unwrap();
        if events.iter().any(|r| r.sse_kind.as_deref() == Some(kind)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("event kind {kind:?} never persisted for {sid}");
}

/// Poll until the persisted session agent matches `want`.
async fn wait_agent(store: &Arc<dyn Store>, sid: &str, want: &str) {
    for _ in 0..200 {
        let meta = store.get_session(sid).await.unwrap().unwrap();
        if meta.agent.as_deref() == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("session {sid} never reached agent {want:?}");
}

fn seeded_state(
    tmp: &tempfile::TempDir,
    store: &Arc<dyn Store>,
    llm: Arc<HangingStream>,
) -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        store: store.clone(),
        workdir: tmp.path().to_path_buf(),
        handles: opencoder_web::handle::new_handle_map(),
        client_override: Some(llm),
    })
}

async fn seed_session(store: &Arc<dyn Store>, sid: &str) {
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            // Seeded title: a successful run would otherwise fire
            // maybe_generate_title, which hangs on the HangingStream and keeps
            // `draining` true for up to 30 s.
            title: Some("seeded".into()),
            agent: Some("act".into()),
            model: Some("test/model".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// The admission-time mode changes stay 409 while running, side-effect free;
/// after the drain ends the same switch succeeds.
#[tokio::test]
async fn agent_field_and_dedicated_switch_paths_remain_409_while_running() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "running-mode";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;

    for (uri, body) in [
        (format!("/api/sessions/{sid}/agent"), r#"{"value":"plan"}"#),
        (
            format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"ordinary text","agent":"plan","delivery":"queue"}"#,
        ),
        (format!("/api/sessions/{sid}/handoff"), r#"{"extra":"now"}"#),
    ] {
        let response = request(&app, "POST", &uri, body).await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{uri} accepted");
    }

    // Rejected paths leave zero footprint.
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
    for delivery in [Delivery::Steer, Delivery::Queue] {
        let pending = store.pending_inputs(sid, delivery).await.unwrap();
        assert!(
            pending.iter().all(|input| input.prompt != "ordinary text"),
            "rejected agent-field request was admitted: {pending:?}"
        );
    }

    // Stop the run; the same switch succeeds at the idle boundary.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;
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

/// A textual mode command + queue is admitted while running (200): the item
/// sits in the queue, the skill persists, the agent is untouched until the
/// runner consumes the queue at the idle boundary of the next drain.
#[tokio::test]
async fn queued_mode_command_admitted_while_running_applies_at_idle_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "queued-mode";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;

    // Mode command + queue + skill while the drain runs: admitted now.
    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/plan review","delivery":"queue","skill":"reviewer"}"#,
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "queued mode command must be admitted while running"
    );

    // Skill persisted at admission; agent not yet applied (still running).
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"));
    assert_eq!(meta.skill.as_deref(), Some("reviewer"));

    // The queued mode command sits pending, waiting for the idle boundary.
    let pending = store.pending_inputs(sid, Delivery::Queue).await.unwrap();
    assert!(
        pending.iter().any(|i| i.prompt == "/plan review"),
        "queued /plan review must be pending: {pending:?}"
    );

    // Stop the hung run; the queue row survives.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;

    // A fresh drain consumes the queue: /plan applies at the idle boundary
    // (agent flip persisted before the LLM turn), then "review" runs.
    let kick = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"kickoff","delivery":"queue"}"#,
    )
    .await;
    assert_eq!(kick.status(), StatusCode::OK);

    wait_agent(&store, sid, "plan").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("plan"));

    // Cleanup: stop the drain parked on the "review" LLM turn.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;
}

/// A textual mode command + steer while running is admitted (200), absorbed
/// at the turn boundary (turn-cancel interrupts the in-flight turn) and
/// applied — AgentSwitch + done, no LLM call, no command leak.
#[tokio::test]
async fn steered_mode_command_admitted_while_running_applies_at_turn_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "steered-mode";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;

    // Bare "/plan" steer while the drain is stuck in the LLM call: 200.
    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/plan","delivery":"steer"}"#,
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "steered mode command must be admitted while running"
    );

    // The steer fires turn_cancel; the runner absorbs it at the next turn
    // boundary and goes idle (bare command, no LLM call).
    wait_idle(&state, sid).await;
    wait_agent(&store, sid, "plan").await;
    wait_for_event_kind(&store, sid, "steer_consumed").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;
    wait_for_event_kind(&store, sid, "done").await;

    // Exactly one LLM call (the original hung turn); the command never
    // reaches the transcript.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "no second LLM call");
    let messages = store.load_messages(sid).await.unwrap();
    assert!(messages
        .iter()
        .all(|message| !message.text().contains("/plan")));
    assert!(store
        .pending_inputs(sid, Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}
