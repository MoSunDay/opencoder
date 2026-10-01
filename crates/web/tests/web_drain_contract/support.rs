use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use opencoder_core::ContentBlock;
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

/// Fresh in-memory AppState (drain tests call handlers/fns directly, no router).
pub(super) async fn state() -> Arc<opencoder_web::AppState> {
    state_with_workdir(std::env::temp_dir()).await
}

/// AppState backed by an in-memory store but a custom workdir (for tests that
/// need to place config files on disk).
pub(super) async fn state_with_workdir(
    workdir: std::path::PathBuf,
) -> Arc<opencoder_web::AppState> {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    Arc::new(opencoder_web::AppState {
        config_home: None,
        client_override: None,
        brain: opencoder_web::api_brain::mock_brain(store.clone()),
        store,
        workdir,
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        team: opencoder_web::team_state::mock(),
        project: opencoder_web::ProjectService::new(),
    })
}

/// Seed a session row (default agent "act", model "m").
pub(super) async fn seed(state: &opencoder_web::AppState, sid: &str) {
    state
        .store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.to_string(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            kind: None,
        })
        .await
        .unwrap();
}

/// Mock that completes a single assistant turn replying `text`.
pub(super) fn mock_reply(text: &str) -> Arc<dyn ChatStream> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

/// Admit a prompt and spawn its drain, returning the admitted seq. Wraps the
/// production `admit_and_drain` so each test stays focused on the contract.
pub(super) async fn admit(
    state: &opencoder_web::AppState,
    sid: &str,
    prompt: &str,
    reply: &str,
) -> i64 {
    opencoder_web::handle::admit_and_drain(
        state.handles.clone(),
        state.store.clone(),
        sid,
        prompt.to_string(),
        Vec::new(),
        opencoder_store::Delivery::Steer,
        mock_reply(reply),
        std::env::temp_dir(),
        opencoder_core::Config {
            model: "m/g".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// True once an assistant Text block containing `needle` is persisted.
async fn replied(state: &opencoder_web::AppState, sid: &str, needle: &str) -> bool {
    state
        .store
        .load_messages(sid)
        .await
        .unwrap()
        .iter()
        .flat_map(|m| m.blocks.iter())
        .any(|b| matches!(b, ContentBlock::Text { text } if text.contains(needle)))
}

/// Poll until `replied` holds or ~3s elapse.
pub(super) async fn eventually_replied(
    state: &opencoder_web::AppState,
    sid: &str,
    needle: &str,
) -> bool {
    for _ in 0..120 {
        if replied(state, sid, needle).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// Poll until the session's drain is idle (`draining` reset). The handle stays
/// in the map after completion, so this also asserts the DrainGuard ran.
pub(super) async fn wait_idle(state: &opencoder_web::AppState, sid: &str) {
    for _ in 0..120 {
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
