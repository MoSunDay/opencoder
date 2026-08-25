//! Client↔server tests for the `remote_ops` extension methods, restricted to
//! endpoints already merged in the web crate when this file was written
//! (fork / compact / handoff / skill / config / get+delete session). The
//! question/input/annotation/autopilot endpoints are covered once the server
//! side lands (see client_ops dispatch); the pure client-side logic (retry
//! policy, workdir resolution, autopilot validation) has unit tests in the
//! client/cli crates instead (test pyramid: no server → no integration here).

use std::sync::Arc;

use opencoder_client::Remote;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store, SubagentStatus, SubagentTaskRecord};

const TOKEN: &str = "remote-ops-token";

async fn state_with_mock() -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap().keep();
    let mock: Arc<dyn ChatStream> =
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
    Arc::new(opencoder_web::AppState {
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        client_override: Some(mock),
    })
}

async fn spawn_server(state: Arc<opencoder_web::AppState>) -> String {
    let app = opencoder_web::build_app(state, Some(TOKEN.into()), true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn fork_then_delete_session_roundtrip() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let id = remote.create_session(None, None).await.unwrap();
    let forked = remote.fork_session(&id).await.unwrap();
    assert!(!forked.is_empty());
    assert_ne!(forked, id, "fork must produce a distinct session id");

    // Both appear in the listing.
    let list = remote.list_sessions(None, None, None).await.unwrap();
    for want in [&id, &forked] {
        assert!(
            list.iter()
                .any(|v| v.get("id").and_then(|i| i.as_str()) == Some(want.as_str())),
            "session {want} missing from listing"
        );
    }

    // GET /api/sessions/:id returns the resource JSON (id + messages + meta).
    // The endpoint is lenient by design: an unknown id yields an empty
    // resource, not a 404 — the listing above is the existence check.
    let got = remote.get_session(&forked).await.unwrap();
    assert!(got.is_object(), "session resource must be a JSON object");
    assert!(
        got.get("messages").and_then(|m| m.as_array()).is_some(),
        "session resource must carry a messages array: {got}"
    );

    remote.delete_session(&forked).await.unwrap();
    let err = remote
        .delete_session(&forked)
        .await
        .expect_err("deleting a deleted session must fail");
    assert!(
        err.to_string().contains("404"),
        "second delete must surface the server's 404: {err:#}"
    );
}

#[tokio::test]
async fn compact_handoff_and_skill_are_accepted() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();
    let id = remote.create_session(Some("plan"), None).await.unwrap();

    // compact: 202 Accepted → Ok(()) (the drain runs async; not awaited here).
    remote.post_compact(&id).await.unwrap();

    // handoff with extra guidance text.
    remote.post_handoff(&id, "focus on tests").await.unwrap();

    // skill set/clear round-trip (current client API: Result<()>; the
    // endpoint's {"ok":true} is asserted at the web layer elsewhere).
    remote.post_skill(&id, Some("task-plan")).await.unwrap();
    remote.post_skill(&id, None).await.unwrap();

    // unknown session: the server error body must reach the caller.
    let err = remote.post_compact("no-such-session").await.unwrap_err();
    assert!(
        err.to_string().contains("404"),
        "compact on missing session must surface 404: {err:#}"
    );
}

#[tokio::test]
async fn config_get_returns_redacted_object() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let cfg = remote.get_config().await.unwrap();
    assert!(cfg.is_object(), "config must be a JSON object");
    // api_key fields must never come back over the wire.
    let raw = cfg.to_string();
    assert!(
        !raw.contains("\"api_key\":\"") || raw.contains("REDACTED") || raw.contains("***"),
        "secrets must be redacted in the config payload"
    );
}

#[tokio::test]
async fn questions_list_empty_and_unknown_answer_404() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();
    let id = remote.create_session(None, None).await.unwrap();

    // No question is waiting on a fresh session → empty list, not an error.
    let questions = remote.list_questions(&id).await.unwrap();
    assert!(questions.is_empty());

    // Answering/skipping an unknown call id must surface the server's 404.
    let err = remote
        .answer_question(&id, "call-404", "yes")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("404"), "answer: {err:#}");
    let err = remote.skip_question(&id, "call-404").await.unwrap_err();
    assert!(err.to_string().contains("404"), "skip: {err:#}");
}

#[tokio::test]
async fn inputs_list_and_delivery_validation() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();
    let id = remote.create_session(None, None).await.unwrap();

    // Fresh session has no pending inputs in either lane.
    assert!(remote.list_inputs(&id, "steer").await.unwrap().is_empty());
    assert!(remote.list_inputs(&id, "queue").await.unwrap().is_empty());

    // Invalid delivery is a 400 from the server — the error must carry it.
    let err = remote.list_inputs(&id, "bogus").await.unwrap_err();
    assert!(
        err.to_string().contains("400"),
        "invalid delivery must 400: {err:#}"
    );
}

#[tokio::test]
async fn annotation_and_autopilot_roundtrip() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();
    let id = remote.create_session(None, None).await.unwrap();

    // Annotation set → echoed back; clear (None) → null.
    let v = remote
        .post_annotation(&id, Some("must have tests"))
        .await
        .unwrap();
    assert_eq!(v.get("ok").and_then(|o| o.as_bool()), Some(true));
    assert_eq!(
        v.get("requirement").and_then(|r| r.as_str()),
        Some("must have tests")
    );
    let v = remote.post_annotation(&id, None).await.unwrap();
    assert_eq!(v.get("requirement"), Some(&serde_json::Value::Null));

    // Autopilot set → mode echoed; invalid mode → 400; clear → null.
    let v = remote.post_autopilot(&id, Some("ap")).await.unwrap();
    assert_eq!(v.get("ok").and_then(|o| o.as_bool()), Some(true));
    assert_eq!(v.get("mode").and_then(|m| m.as_str()), Some("ap"));
    let err = remote.post_autopilot(&id, Some("bogus")).await.unwrap_err();
    assert!(err.to_string().contains("400"), "autopilot: {err:#}");
    let v = remote.post_autopilot(&id, None).await.unwrap();
    assert_eq!(v.get("mode"), Some(&serde_json::Value::Null));

    // Both endpoints 404 on a missing session.
    let err = remote
        .post_annotation("no-such-session", Some("x"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("404"), "annotation 404: {err:#}");
    let err = remote
        .post_autopilot("no-such-session", Some("ap"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("404"), "autopilot 404: {err:#}");
}

#[tokio::test]
async fn models_and_skills_catalogs_are_objects() {
    let state = state_with_mock().await;
    let base = spawn_server(state).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let models = remote.get_models().await.unwrap();
    assert!(models.get("default").and_then(|d| d.as_str()).is_some());
    assert!(
        models
            .get("models")
            .and_then(|m| m.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "models list must be non-empty: {models}"
    );

    let skills = remote.get_skills().await.unwrap();
    assert!(skills.get("skills").and_then(|s| s.as_array()).is_some());
}

#[tokio::test]
async fn list_subagents_and_clear_sessions_roundtrip() {
    let state = state_with_mock().await;
    let base = spawn_server(state.clone()).await;
    let remote = Remote::new(&base, TOKEN).unwrap();

    let keep = remote.create_session(None, None).await.unwrap();
    let other = remote.create_session(None, None).await.unwrap();
    let child = remote.create_session(None, None).await.unwrap();
    state
        .store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "task-1".into(),
            parent_session_id: other.clone(),
            child_session_id: child.clone(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "scan the crates".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    // Unknown parent surfaces the server's 404; an existing parent with no
    // tasks is a normal empty list.
    let err = remote
        .list_subagents("no-such-session")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("404"), "list 404: {err:#}");
    assert!(
        remote.list_subagents(&keep).await.unwrap().is_empty(),
        "keep session has no tasks yet"
    );

    let tasks = remote.list_subagents(&other).await.unwrap();
    assert_eq!(tasks.len(), 1, "tasks: {tasks:?}");
    assert_eq!(tasks[0]["id"], "task-1");
    assert_eq!(tasks[0]["kind"], "explore");
    assert_eq!(tasks[0]["status"], "running");
    assert_eq!(tasks[0]["child_session_id"], child);
    assert_eq!(tasks[0]["prompt"], "scan the crates");

    let removed = remote.clear_sessions(&keep).await.unwrap();
    assert_eq!(removed, 2, "other + its subagent child cleared");
    assert!(state.store.get_session(&keep).await.unwrap().is_some());
    assert!(state.store.get_session(&other).await.unwrap().is_none());
    assert!(state.store.get_session(&child).await.unwrap().is_none());
    assert!(
        state.store.get_subagent_task("task-1").await.unwrap().is_none(),
        "subagent task rows cascade with their parent session"
    );
    assert_eq!(
        remote.clear_sessions(&keep).await.unwrap(),
        0,
        "clear is idempotent"
    );
    let err = remote.clear_sessions("no-such-session").await.unwrap_err();
    assert!(err.to_string().contains("404"), "clear 404: {err:#}");
}
