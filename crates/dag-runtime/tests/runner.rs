#![cfg(unix)]
use opencoder_core::{
    harness::{CodexSettings, RunnerSettings, Versioned},
    Config,
};
use opencoder_dag::{DagSpec, StepOutcome};
use opencoder_dag_runtime::exec::{runner, ExecDeps, StepCtx};
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

async fn fixture(root: &Path, mode: &str) -> (StepCtx, ExecDeps) {
    let agents = root.join("agents");
    std::fs::create_dir_all(agents.join("act")).unwrap();
    std::fs::write(
        agents.join("act/meta.json"),
        r#"{"name":"act","harness":"codex","harness_profile":"business"}"#,
    )
    .unwrap();
    let exe = root.join("runner.py");
    std::fs::write(&exe, include_str!("runner/fixture.py")).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = Config::default();
    config.agent.agents_dir = Some(agents);
    let settings: RunnerSettings = serde_json::from_value(json!({"command":[exe],"workdir":root,
        "files":{exe.display().to_string():runner::artifacts::checksum(&exe).unwrap()},"envs":{"MODE":format!("fixture:{mode}")}})).unwrap();
    config.agent.runtime.runners.insert(
        "business".into(),
        Versioned {
            revision: 3,
            settings,
        },
    );
    let settings: CodexSettings = serde_json::from_value(json!({"executable":exe,"model":"pinned-model","auth_slot":1,"envs":{"PRIVATE":"private-profile-value"}})).unwrap();
    config.agent.runtime.profiles.insert(
        "business".into(),
        Versioned {
            revision: 4,
            settings,
        },
    );
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "dag-business".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let spec: DagSpec = serde_json::from_value(
        json!({"name":"business", "steps":[{"name":"workflow","timeout_secs":2,
        "kind":{"type":"runner","runner":"business","agent":"act"}}]}),
    )
    .unwrap();
    let run = root.join("dag-business");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(
        run.join("input.json"),
        json!({"job_id":"job-fixture","attempt":1}).to_string(),
    )
    .unwrap();
    let ctx = StepCtx {
        run_id: "dag-business".into(),
        step: spec.steps[0].clone(),
        spec,
        states: Default::default(),
        outputs: Default::default(),
        workflow_root: root.to_owned(),
    };
    let deps = ExecDeps {
        store,
        client: Arc::new(MockChatClient::new()),
        workdir: root.to_owned(),
        config,
    };
    (ctx, deps)
}

#[tokio::test]
async fn runner_persists_codex_transcript_artifacts_and_recovers_without_reexecution() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, deps) = fixture(dir.path(), "normal").await;
    let result = runner::execute(&ctx, &deps, CancellationToken::new()).await;
    assert_eq!(result.outcome, StepOutcome::Done, "{:?}", result.error);
    assert_eq!(
        result.output_json.as_ref().unwrap()["result"]["verdict"],
        "block"
    );
    let messages = deps.store.load_messages(&ctx.run_id).await.unwrap();
    let text = serde_json::to_string(&messages).unwrap();
    for expected in ["执行阶段", "inspect first", "tool result", "answer"] {
        assert!(text.contains(expected), "{text}");
    }
    assert!(!text.contains("private-profile-value"));
    assert!(messages.iter().any(|m| m.usage.output_tokens == 5));
    let config: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("invocation.json")).unwrap())
            .unwrap();
    assert_eq!(config["codex"]["model"], "pinned-model");
    assert_eq!(config["profile_revision"], 4);
    assert_eq!(config["runner_revision"], 3);
    assert_eq!(config["codex"]["auth_slot"], 1);
    assert!(Path::new(config["agent"]["resources"].as_str().unwrap()).is_dir());
    let restored = runner::execute(&ctx, &deps, CancellationToken::new()).await;
    assert_eq!(restored.outcome, StepOutcome::Done);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("starts")).unwrap(),
        "1\n"
    );
    std::fs::write(
        dir.path()
            .join("dag-business/workflow/artifacts/result.json"),
        "{}",
    )
    .unwrap();
    let corrupted = runner::execute(&ctx, &deps, CancellationToken::new()).await;
    assert_eq!(corrupted.outcome, StepOutcome::Error);
    assert!(corrupted.error.unwrap().contains("checksum"));
}

#[tokio::test]
async fn runner_rejects_bad_events_incomplete_codex_and_bad_artifacts_without_retry() {
    for (mode, reason) in [
        ("invalid", "invalid Runner event"),
        ("unfinished", "unfinished Codex"),
        ("tamper", "checksum"),
        ("missing", "without a result"),
        ("failure", "Runner:"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, deps) = fixture(dir.path(), mode).await;
        let result = runner::execute(&ctx, &deps, CancellationToken::new()).await;
        assert_eq!(result.outcome, StepOutcome::Error, "mode={mode}");
        assert!(
            result.error.as_ref().unwrap().contains(reason),
            "mode={mode} error={:?}",
            result.error
        );
        let restored = runner::execute(&ctx, &deps, CancellationToken::new()).await;
        assert!(restored.error.unwrap().contains("new business attempt"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("starts")).unwrap(),
            "1\n"
        );
    }
}

#[tokio::test]
async fn cancellation_and_timeout_reap_runner_descendants() {
    for cancelled in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, deps) = fixture(dir.path(), "hang").await;
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let pid_file = dir.path().join("child.pid");
        let wait_file = pid_file.clone();
        let task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !wait_file.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if cancelled {
                trigger.cancel();
            }
        });
        let result = runner::execute(&ctx, &deps, cancel).await;
        task.await.unwrap();
        assert_eq!(
            result.outcome,
            if cancelled {
                StepOutcome::Cancelled
            } else {
                StepOutcome::Error
            }
        );
        let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("Runner descendant must be reaped");
    }
}
