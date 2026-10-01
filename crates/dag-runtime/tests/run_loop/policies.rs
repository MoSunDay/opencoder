use super::*;

#[tokio::test]
async fn invalid_spec_snapshot_fails_before_scheduling() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new());
    let f = fixture(&base, &tmp, client.clone()).await;

    // Cycle: never dispatchable, but constructible in-memory — the runtime
    // must fold it into a clean error report instead of wedging.
    let spec = DagSpec {
        max_concurrency: 4,
        name: "e2e-cycle".into(),
        description: None,
        steps: vec![agent_step("a", &["b"], None), agent_step("b", &["a"], None)],
    };
    let run = claimed(spec);
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&f.uplink),
            exec: ExecDeps {
                store: Arc::clone(&f.store),

                workdir: f.workdir.clone(),
                config: f.config.clone(),
            },
            workflow_root: f.workflow_root.clone(),
        },
        run,
        cancel_rx,
    )
    .await
    .unwrap();

    assert_eq!(status, DagRunStatus::Error);
    let report = await_status(&shared).await;
    assert_eq!(report.status, "error");
    assert!(report.error.unwrap().contains("invalid spec"));
    // No scheduling happened: only the terminal frame, nothing per-step.
    let c = shared.lock().unwrap();
    assert_eq!(kinds(&c), vec!["run_finished"]);
}

#[tokio::test]
async fn pre_cancelled_run_folds_cancelled_without_scheduling() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new());
    let f = fixture(&base, &tmp, client.clone()).await;

    let run = claimed(one_step_spec());
    let (tx, cancel_rx) = tokio::sync::watch::channel(false);
    tx.send(true).unwrap();
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&f.uplink),
            exec: ExecDeps {
                store: Arc::clone(&f.store),

                workdir: f.workdir.clone(),
                config: f.config.clone(),
            },
            workflow_root: f.workflow_root.clone(),
        },
        run,
        cancel_rx,
    )
    .await
    .unwrap();

    assert_eq!(status, DagRunStatus::Cancelled);
    let report = await_status(&shared).await;
    assert_eq!(report.status, "cancelled");
    let c = shared.lock().unwrap();
    // run_started still leads (emitted before the loop head checks cancel),
    // but no step ever started; the step folds to a cancelled step_done.
    assert_eq!(kinds(&c), vec!["run_started", "step_done", "run_finished"]);
    assert_eq!(c.events[1].step.as_deref(), Some("analyze"));
    assert_eq!(c.events[1].payload["ok"], json!(false));
}

/// Regression (in-flight re-dispatch): two dependency-free agent steps run
/// concurrently; `ready_steps` cannot see in-flight steps (their outcome is
/// not in `states` yet), so completing `fast` must not re-spawn the still
/// running `slow` (duplicate sessions / events / artifacts). The third mock
/// script is a canary only a buggy re-dispatch would consume.
#[tokio::test]
async fn status_delivery_failure_is_returned_to_the_execution_owner() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/api/nodes/dag/runs/:rid/events",
            post(|| async { StatusCode::OK }),
        )
        .route(
            "/api/nodes/dag/runs/:rid/status",
            post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "fixture status persistence unavailable",
                )
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let f = fixture(&base, &tmp, Arc::new(MockChatClient::new())).await;
    let run = claimed(DagSpec {
        max_concurrency: 4,
        name: "invalid-cycle".into(),
        description: None,
        steps: vec![agent_step("a", &["b"], None), agent_step("b", &["a"], None)],
    });
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let result = execute_run(
        RunDeps {
            uplink: f.uplink,
            exec: ExecDeps {
                store: f.store,

                workdir: f.workdir,
                config: f.config,
            },
            workflow_root: f.workflow_root,
        },
        run,
        cancel_rx,
    )
    .await;
    server.abort();
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("terminal status delivery failed"), "{error}");
    assert!(
        error.contains("fixture status persistence unavailable"),
        "{error}"
    );
}
