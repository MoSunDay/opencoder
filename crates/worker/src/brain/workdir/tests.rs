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
            config: config.clone(),
            command: None,
        })),
        result: json!({}),
        error: None,
        events: vec![],
        lifecycle: Lifecycle::default(),
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .save(record.clone())
        .unwrap();
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
    let (state, ordinary_config) = native_state(&worker, "/api/sessions/ordinary/prompt")
        .await
        .unwrap();
    assert_eq!(state.workdir, worker.inner.state.workdir);
    assert!(ordinary_config.is_none());
    // The matrix includes historical Brain Operators: their admitted config
    // and isolated workspace must survive introduction of versioned isolation.
    for (suffix, kind, managed, isolated) in [
        ("normal-agent", ExecutionKind::Agent, false, false),
        ("legacy-operator", ExecutionKind::Operator, false, false),
        (
            "legacy-brain-operator",
            ExecutionKind::Operator,
            true,
            false,
        ),
        ("operator", ExecutionKind::Operator, false, true),
        ("brain-operator", ExecutionKind::Operator, true, true),
    ] {
        let mut item = record.clone();
        item.assignment.index.id = suffix.into();
        item.assignment.index.kind = kind;
        item.assignment.request.id = suffix.into();
        item.assignment.request.kind = kind;
        if !managed {
            item.assignment.request.input = json!({});
        }
        let snapshot_home = if isolated {
            item.annotations = json!({"operator_environment_version":1});
            let mut frozen = config.clone();
            frozen.model = "fixture/frozen-model".into();
            let (home, _) = crate::operations::operator_env::materialize(
                &worker.inner.layout,
                kind,
                suffix,
                &frozen,
            )
            .unwrap()
            .unwrap();
            Some(home)
        } else {
            None
        };
        worker
            .inner
            .journal
            .lock()
            .await
            .save(item.clone())
            .unwrap();
        for action in ["prompt", "compact", "handoff"] {
            let (state, override_config) =
                native_state(&worker, &format!("/api/sessions/{suffix}/{action}"))
                    .await
                    .unwrap();
            assert_eq!(state.config_home, snapshot_home);
            assert_eq!(state.workdir, for_record(&worker, &item).unwrap());
            let resumed = execution_config(&worker, &item).unwrap();
            if isolated {
                assert!(override_config.is_none());
                assert_eq!(resumed.unwrap().model, "fixture/frozen-model");
                assert_eq!(
                    state.workdir,
                    worker.inner.layout.workspace_dir(kind, suffix).unwrap()
                );
            } else if managed {
                assert_eq!(override_config.unwrap().model, "fixture/admitted-model");
                assert_eq!(resumed.unwrap().model, "fixture/admitted-model");
                assert_ne!(state.workdir, worker.inner.state.workdir);
            } else {
                assert!(override_config.is_none());
                assert!(resumed.is_none());
                assert_eq!(state.workdir, worker.inner.state.workdir);
            }
        }
        if let Some(home) = snapshot_home {
            std::fs::remove_file(home.join(".opencoder/config.json")).unwrap();
            assert!(
                native_state(&worker, &format!("/api/sessions/{suffix}/prompt"))
                    .await
                    .is_err()
            );
            assert!(execution_config(&worker, &item).is_err());
            assert!(for_record(&worker, &item).is_err());
        }
    }
    worker.inner.stopping.cancel();
    drop(admission);
    worker.shutdown().await.unwrap();
}
