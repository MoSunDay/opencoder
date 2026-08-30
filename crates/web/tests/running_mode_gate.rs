//! End-to-end HTTP contract for the running mode gate against the real Axum
//! router and drain lifecycle, with a deliberately hanging LLM stream.
//!
//! Contract split:
//! - Admission-time mode changes (the `agent` field, POST /agent, POST
//!   /handoff) stay 409 while draining — they rewrite session config.
//! - Textual control commands (/sandbox, /act, /act_clear_context) are ADMITTED
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
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
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

/// Same seeding as [`seed_session`] but the session starts as the sandbox
/// agent — the convergence target for clear-context is act.
async fn seed_sandbox_session(store: &Arc<dyn Store>, sid: &str) {
    store
        .create_session(&SessionMeta {
            id: sid.into(),
            // Seeded title: prevents maybe_generate_title from hanging on the
            // HangingStream (same rationale as seed_session).
            title: Some("seeded".into()),
            agent: Some("sandbox".into()),
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
        (format!("/api/sessions/{sid}/agent"), r#"{"value":"sandbox"}"#),
        (
            format!("/api/sessions/{sid}/prompt"),
            r#"{"prompt":"ordinary text","agent":"sandbox","delivery":"queue"}"#,
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
        r#"{"value":"sandbox"}"#,
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
        Some("sandbox")
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
        r#"{"prompt":"/sandbox review","delivery":"queue","skill":"reviewer"}"#,
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
        pending.iter().any(|i| i.prompt == "/sandbox review"),
        "queued /sandbox review must be pending: {pending:?}"
    );

    // Stop the hung run; the queue row survives.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;

    // A fresh drain consumes the queue: /sandbox applies at the idle boundary
    // (agent flip persisted before the LLM turn), then "review" runs.
    let kick = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"kickoff","delivery":"queue"}"#,
    )
    .await;
    assert_eq!(kick.status(), StatusCode::OK);

    wait_agent(&store, sid, "sandbox").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("sandbox"));

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

    // Bare "/sandbox" steer while the drain is stuck in the LLM call: 200.
    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/sandbox","delivery":"steer"}"#,
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
    wait_agent(&store, sid, "sandbox").await;
    wait_for_event_kind(&store, sid, "steer_consumed").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;
    wait_for_event_kind(&store, sid, "done").await;

    // Exactly one LLM call (the original hung turn); the command never
    // reaches the transcript.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "no second LLM call");
    let messages = store.load_messages(sid).await.unwrap();
    assert!(messages
        .iter()
        .all(|message| !message.text().contains("/sandbox")));
    assert!(store
        .pending_inputs(sid, Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}

/// A steered `/clear_context` while running is admitted (200), absorbed at the
/// turn boundary and applied: `transcript_reset` persists, NO `agent_switched`
/// (this session is already act — the already-act clear is a pure agent
/// no-op; only a sandbox session converges to act) and no legacy
/// `plan_handoff` frame. With no assistant reply yet the boundary degrades to
/// the blank sentinel, so the run stops without a second LLM call.
#[tokio::test]
async fn steered_clear_context_on_act_session_keeps_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "steered-clear";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;
    // Everything persisted up to here is pre-command.
    let cursor = store.last_event_seq(sid).await.unwrap();

    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/clear_context","delivery":"steer"}"#,
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "steered control command must be admitted while running"
    );

    wait_idle(&state, sid).await;
    wait_for_event_kind(&store, sid, "transcript_reset").await;
    wait_for_event_kind(&store, sid, "steer_consumed").await;

    let events = store.events_after(sid, cursor).await.unwrap();
    assert!(
        !events
            .iter()
            .any(|r| r.sse_kind.as_deref() == Some("agent_switched")),
        "clear-context must keep the active agent (no agent_switched): {:?}",
        events.iter().filter_map(|r| r.sse_kind.clone()).collect::<Vec<_>>()
    );
    assert!(
        !events
            .iter()
            .any(|r| r.sse_kind.as_deref() == Some("plan_handoff")),
        "the legacy plan_handoff frame is gone; got {:?}",
        events.iter().filter_map(|r| r.sse_kind.clone()).collect::<Vec<_>>()
    );

    // No assistant reply existed, so the boundary persisted the blank
    // fresh-start sentinel (`handoff_plan` = clear-context marker).
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("act"), "agent unchanged");
    assert!(opencoder_session::control_cmd::is_clear_context_handoff(
        meta.handoff_plan.as_deref().unwrap_or("")
    ), "blank clear must persist the sentinel, got {:?}", meta.handoff_plan);

    // Sentinel path stops without another LLM call; the command never
    // reaches the transcript.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "no second LLM call");
    let messages = store.load_messages(sid).await.unwrap();
    assert!(messages
        .iter()
        .all(|message| !message.text().contains("/clear_context")));
    assert!(store
        .pending_inputs(sid, Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}

/// GET `uri` and return (status, parsed JSON body).
async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// Regression (plan -> sandbox rename): `POST /agent` accepts the `sandbox`
/// agent — the persisted meta and the subsequent GET both reflect it — while
/// the legacy `plan` name (`resolve_agent("plan")` is None) gets the standard
/// unknown-agent 400 and leaves zero footprint.
#[tokio::test]
async fn agent_switch_accepts_sandbox_and_rejects_legacy_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "switch-sandbox";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    // sandbox: accepted, persisted, and visible on the next GET.
    let switched = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/agent"),
        r#"{"value":"sandbox"}"#,
    )
    .await;
    assert_eq!(
        switched.status(),
        StatusCode::OK,
        "sandbox must be a switchable agent"
    );
    assert_eq!(
        store
            .get_session(sid)
            .await
            .unwrap()
            .unwrap()
            .agent
            .as_deref(),
        Some("sandbox")
    );
    let (status, body) = get_json(&app, &format!("/api/sessions/{sid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["meta"]["agent"], "sandbox",
        "GET must report the sandbox agent: {body}"
    );

    // The legacy name is not an agent anymore: standard unknown-agent 400.
    let legacy = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/agent"),
        r#"{"value":"plan"}"#,
    )
    .await;
    assert_eq!(
        legacy.status(),
        StatusCode::BAD_REQUEST,
        "plan must not be switchable"
    );
    let bytes = axum::body::to_bytes(legacy.into_body(), 4096).await.unwrap();
    let err: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(err["ok"], false);
    assert!(
        err["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown agent"),
        "400 must name the unknown agent, got: {err}"
    );

    // Zero footprint: rejected switch left meta + live override untouched.
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(meta.agent.as_deref(), Some("sandbox"));
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    assert_eq!(
        handle.overrides.lock().await.agent.as_deref(),
        Some("sandbox")
    );
}

/// Regression (wire contract): the events a sandbox session emits include an
/// `agent_switched` frame whose value is "sandbox" — never the legacy "plan"
/// value — so reconnect replays render the real agent name.
#[tokio::test]
async fn sandbox_session_emits_agent_switched_with_sandbox_value() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "sandbox-frame";
    seed_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;
    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/sandbox","delivery":"steer"}"#,
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "steered /sandbox must be admitted while running"
    );

    wait_idle(&state, sid).await;
    wait_agent(&store, sid, "sandbox").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;

    let events = store.events_after(sid, 0).await.unwrap();
    let switched: Vec<_> = events
        .iter()
        .filter(|r| r.sse_kind.as_deref() == Some("agent_switched"))
        .collect();
    assert!(
        !switched.is_empty(),
        "an agent_switched frame must be persisted: {:?}",
        events.iter().map(|r| r.sse_kind.clone()).collect::<Vec<_>>()
    );
    for record in &switched {
        assert_eq!(
            record.payload.get("agent").and_then(|v| v.as_str()),
            Some("sandbox"),
            "agent_switched payload must name sandbox: {}",
            record.payload
        );
    }
    assert!(
        events
            .iter()
            .all(|r| r.payload.get("agent").and_then(|v| v.as_str()) != Some("plan")),
        "no frame may carry the legacy plan value"
    );

    // Cleanup: the original drain is parked on the hanging LLM turn.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;
}

/// A queued `/act_clear_context` in a SANDBOX session converges to act: after
/// the drain picks it up the boundary persists, `agent_switched` follows
/// `transcript_reset`, the meta agent is `act`, and the sentinel boundary
/// stops without a second LLM call (no assistant reply existed).
#[tokio::test]
async fn queued_clear_context_in_sandbox_session_converges_to_act() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let sid = "queued-sandbox-clear";
    seed_sandbox_session(&store, sid).await;
    let llm = Arc::new(HangingStream {
        calls: AtomicUsize::new(0),
    });
    let state = seeded_state(&tmp, &store, llm.clone());
    let app = opencoder_web::build_app(state.clone(), None, false);

    start_running_drain(&app, &store, sid, &llm).await;
    // Everything persisted up to here is pre-command.
    let cursor = store.last_event_seq(sid).await.unwrap();

    let admitted = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/act_clear_context","delivery":"queue"}"#,
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "queued control command must be admitted while running"
    );

    // A queued item waits for the idle boundary; hard-cancel the hung run
    // (a hard interrupt keeps pending rows for the next drain), then wake the
    // runner with a bare `/act` (a no-op on this agent-to-be) so the fresh
    // drain pops the queue FIFO and applies the clear without any LLM call.
    let handle = state.handles.lock().await.get(sid).cloned().unwrap();
    handle.cancel.lock().await.cancel();
    wait_idle(&state, sid).await;
    let kick = request(
        &app,
        "POST",
        &format!("/api/sessions/{sid}/prompt"),
        r#"{"prompt":"/act","delivery":"queue"}"#,
    )
    .await;
    assert_eq!(kick.status(), StatusCode::OK);

    wait_agent(&store, sid, "act").await;
    wait_for_event_kind(&store, sid, "transcript_reset").await;
    wait_for_event_kind(&store, sid, "agent_switched").await;
    wait_idle(&state, sid).await;

    // transcript_reset lands before agent_switched (the reset is emitted
    // first; the convergence frame follows it).
    let events = store.events_after(sid, cursor).await.unwrap();
    let reset_idx = events
        .iter()
        .position(|r| r.sse_kind.as_deref() == Some("transcript_reset"))
        .expect("transcript_reset must persist");
    let switch_idx = events
        .iter()
        .position(|r| r.sse_kind.as_deref() == Some("agent_switched"))
        .expect("agent_switched must persist for the sandbox convergence");
    assert!(switch_idx > reset_idx, "agent_switched must follow transcript_reset, got {:?}",
        events.iter().map(|r| r.sse_kind.clone()).collect::<Vec<_>>());

    // The converged agent persists with the boundary.
    let meta = store.get_session(sid).await.unwrap().unwrap();
    assert_eq!(
        meta.agent.as_deref(),
        Some("act"),
        "sandbox clear converges to act"
    );
    assert!(opencoder_session::control_cmd::is_clear_context_handoff(
        meta.handoff_plan.as_deref().unwrap_or("")
    ),
    "the clear (no assistant reply) must persist the sentinel, got {:?}",
    meta.handoff_plan);

    // Sentinel path stops without another LLM call; the command never
    // reaches the transcript.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1, "no second LLM call");
    let messages = store.load_messages(sid).await.unwrap();
    assert!(messages
        .iter()
        .all(|message| !message.text().contains("/act_clear_context")));
    assert!(store
        .pending_inputs(sid, Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}
