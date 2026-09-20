use super::*;
use crate::{lifecycle::Lifecycle, operations::queue::QueuedRun, WorkerOptions};
use opencoder_core::{config::ApMode, fleet::*, Config};
use serde_json::json;

#[tokio::test]
async fn managed_session_keeps_admitted_settings_in_an_isolated_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(directory.path().join("home"));
    let worker = Worker::open(
        WorkerOptions {
            name: "isolated-config".into(),
            workdir: directory.path().join("work"),
            data_dir: directory.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let admission = worker.inner.admission.lock().await;
    let mut config = Config {
        model: "fixture/admitted-model".into(),
        ..Config::default()
    };
    config.provider.base_url = "http://127.0.0.1:12345/v1".into();
    config.autopilot.mode = ApMode::Review;
    let id = "managed-agent";
    let record = Record {
        assignment: Assignment {
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: id.into(),
                kind: ExecutionKind::Agent,
                node_id: worker.inner.registration.id.clone(),
                status: ExecutionStatus::Done,
                created_at: 1,
            },
            request: CreateExecution {
                id: id.into(),
                kind: ExecutionKind::Agent,
                target: Some("act".into()),
                node_id: None,
                input: json!({"brain_scheduler":{"run_id":"root"}}),
            },
            definition: None,
        },
        annotations: json!({}),
        queue: Some(Box::new(QueuedRun {
            ticket: None,
            sequence: 1,
            resume: false,
            config,
            command: None,
        })),
        result: json!({}),
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker.inner.journal.lock().await.save(record).unwrap();
    for action in ["prompt", "compact", "handoff"] {
        let (state, config) = native_state(&worker, &format!("/api/sessions/{id}/{action}"))
            .await
            .unwrap();
        let config = config.expect("managed sessions must retain admitted settings");
        assert_eq!(config.model, "fixture/admitted-model");
        assert_eq!(config.provider.base_url, "http://127.0.0.1:12345/v1");
        assert_eq!(config.autopilot.mode, ApMode::Review);
        assert_ne!(state.workdir, worker.inner.state.workdir);
        assert!(state.workdir.is_dir());
        assert_eq!(std::fs::read_dir(&state.workdir).unwrap().count(), 0);
    }
    let (state, config) = native_state(&worker, "/api/sessions/ordinary/prompt")
        .await
        .unwrap();
    assert_eq!(state.workdir, worker.inner.state.workdir);
    assert!(config.is_none());
    worker.inner.stopping.cancel();
    drop(admission);
    worker.shutdown().await.unwrap();
}
