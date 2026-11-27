use super::{create_session, SessionLabels};
use crate::{Worker, WorkerOptions};
use opencoder_store::{SessionMeta, SessionModelOverrides, TursoStore};

fn labels(title: &str) -> SessionLabels {
    SessionLabels {
        title: Some(title.into()),
        kind: Some("agent".into()),
    }
}

#[tokio::test]
async fn new_sessions_store_empty_mode_overrides_without_rewriting_legacy_rows() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config-home"));
    let workdir = root.path().join("work");
    let worker = Worker::open(
        WorkerOptions {
            name: "agent-session-test".into(),
            workdir: workdir.clone(),
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        None,
    )
    .await
    .unwrap();
    let store = worker.inner.state.store.clone();
    let persisted = TursoStore::open(root.path().join("node/runtime.db"))
        .await
        .unwrap();
    let conn = persisted.conn().await.unwrap();

    store
        .create_session(&SessionMeta {
            id: "legacy".into(),
            kind: Some("agent".into()),
            title: Some("legacy title".into()),
            agent: Some("legacy-agent".into()),
            model: Some("legacy/model".into()),
            created_at: 10,
            updated_at: 20,
            workdir_hash: Some("legacy-workdir".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    create_session(
        &worker,
        "legacy",
        "replacement-agent",
        Some("replacement/model".into()),
        30,
        &workdir,
        labels("replacement title"),
    )
    .await
    .unwrap();

    let legacy = store.get_session("legacy").await.unwrap().unwrap();
    assert_eq!(legacy.title.as_deref(), Some("legacy title"));
    assert_eq!(legacy.agent.as_deref(), Some("legacy-agent"));
    assert_eq!(legacy.model.as_deref(), Some("legacy/model"));
    assert_eq!(legacy.created_at, 10);
    assert_eq!(legacy.updated_at, 20);
    assert_eq!(legacy.workdir_hash.as_deref(), Some("legacy-workdir"));
    assert_eq!(
        store.get_session_model_overrides("legacy").await.unwrap(),
        None
    );

    create_session(
        &worker,
        "fresh",
        "act",
        Some("configured/model".into()),
        40,
        &workdir,
        labels("fresh title"),
    )
    .await
    .unwrap();
    assert_eq!(
        store.get_session_model_overrides("fresh").await.unwrap(),
        Some(SessionModelOverrides::default())
    );

    let mut rows = conn
        .query(
            "SELECT model_overrides_json FROM sessions WHERE id = 'fresh'",
            (),
        )
        .await
        .unwrap();
    let raw: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
    assert_eq!(raw.as_deref(), Some("{}"));

    let mut rows = conn
        .query(
            "SELECT model_overrides_json FROM sessions WHERE id = 'legacy'",
            (),
        )
        .await
        .unwrap();
    let raw: Option<String> = rows.next().await.unwrap().unwrap().get(0).unwrap();
    assert_eq!(raw, None);

    drop(conn);
    drop(persisted);
    worker.shutdown().await.unwrap();
}
