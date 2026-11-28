//! Fork refusals at the switch entry. A Server-bound task has no local
//! transcript to copy: `switch_session(Fork(id))` must refuse before any
//! row is written or handle rebound, and the rendered flash text must keep
//! the whole chain for `Fork` exactly as it does for `Resume`.

use super::load_failure::{assert_switch_refused, current_task, live_config};
use super::*;
use opencoder_core::fleet::ExecutionKind;
use opencoder_core::harness::{HarnessRuntime, RemoteSession, ServerCapability};
use opencoder_store::{SessionFilter, TursoStore};

/// A Server-bound task as the picker lists it: `tui_remote_<kind>` row plus
/// a `HarnessRuntime` carrying the binding (what `remote::create` persists).
async fn remote_bound_task(
    dir: &tempfile::TempDir,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    store: &Arc<dyn Store>,
) -> String {
    let binding = RemoteSession {
        server_url: "http://127.0.0.1:1".into(),
        capability: ServerCapability {
            id: "ops".into(),
            kind: ExecutionKind::Agent,
            target: "codex".into(),
            summary: "Ops".into(),
        },
        created: false,
        initial_input: None,
    };
    let session = crate::remote::create(
        binding,
        config.clone(),
        client.clone(),
        store.clone(),
        dir.path(),
    )
    .await
    .unwrap();
    let runtime: HarnessRuntime = store
        .harness_runtime(&session.id)
        .await
        .unwrap()
        .expect("fixture: the task is bound to its Server");
    assert!(
        runtime
            .remote
            .is_some_and(|remote| remote.capability.id == "ops"),
        "fixture: the binding names the capability"
    );
    session.id
}

async fn remote_session_ids(store: &Arc<dyn Store>) -> Vec<String> {
    let mut ids: Vec<String> = store
        .list_sessions(&SessionFilter {
            kind: Some("tui_remote_agent".into()),
            include_subagents: true,
            ..SessionFilter::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    ids.sort();
    ids
}

/// Forking a Server-bound task is refused by the guard that runs before the
/// copy: no new session row, the source binding untouched, and the current
/// task's runtime/display state exactly as it was.
#[tokio::test]
async fn fork_of_remote_bound_task_is_refused_without_persisting_or_rebinding() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let config = live_config();
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let remote_id = remote_bound_task(&dir, &config, &client, &store).await;
    let before = remote_session_ids(&store).await;
    assert_eq!(before, vec![remote_id.clone()], "fixture: one bound task");
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut task = current_task(&initial, store.clone());

    // The guard text carries no session id; the chain is asserted on the
    // refusal reason only.
    assert_switch_refused(
        &mut task,
        crate::task::TaskPick::Fork(remote_id.clone()),
        dir.path(),
        &config,
        &client,
        &store,
        &["Remote tasks cannot be forked locally"],
    )
    .await;
    assert_eq!(
        remote_session_ids(&store).await,
        before,
        "a refused fork must not persist a copy of the task"
    );
    let runtime = store
        .harness_runtime(&remote_id)
        .await
        .unwrap()
        .expect("the source binding survives the refusal");
    assert_eq!(
        runtime.remote.map(|remote| remote.capability.id),
        Some("ops".into())
    );
}

/// `switch_error` for a `Fork` pick renders the full chain -- the switch
/// context with the picked session id and the underlying cause -- the same
/// way it does for `Resume`; the flash line never collapses it to the tail.
/// The context here is synthetic: the rendering is generic over the pick and
/// does not imply production attaches a fork-specific context today.
#[test]
fn switch_error_for_fork_keeps_session_id_and_cause_in_the_chain() {
    let error = anyhow::Error::new(std::io::Error::other("copy messages failed"))
        .context("task switch fork for session fork-source");
    let text = switch_error(&crate::task::TaskPick::Fork("fork-source".into()), &error);
    assert_eq!(text, format!("{error:#}"));
    assert!(
        text.contains("task switch fork for session fork-source"),
        "the chain still names the picked session: {text}"
    );
    assert!(
        text.contains("copy messages failed"),
        "the source error stays in the chain: {text}"
    );
}
