use super::*;

struct ConcurrentResponses {
    fast: MockChatClient,
    slow: MockChatClient,
}

impl opencoder_llm::ChatStream for ConcurrentResponses {
    fn chat_stream(
        &self,
        request: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        if request
            .messages
            .iter()
            .any(|message| message.text().contains("fast prompt"))
        {
            opencoder_llm::ChatStream::chat_stream(&self.fast, request)
        } else if request
            .messages
            .iter()
            .any(|message| message.text().contains("slow prompt"))
        {
            opencoder_llm::ChatStream::chat_stream(&self.slow, request)
        } else {
            anyhow::bail!("unexpected concurrent step prompt")
        }
    }

    fn backend(&self) -> &'static str {
        "mock"
    }
}

#[tokio::test]
async fn sibling_completion_never_redispatches_inflight_step() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let text = "结论\n```json\n{\"answer\": 1}\n```";
    let completion = || {
        vec![
            LlmEvent::TextDelta(text.into()),
            LlmEvent::Completed {
                text: text.into(),
                tool_calls: vec![],
                usage: None,
            },
        ]
    };
    let hold = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(ConcurrentResponses {
        fast: MockChatClient::new()
            .push_script(completion())
            .push_script(completion()),
        slow: MockChatClient::new()
            .push_hang(Arc::clone(&hold))
            .push_script(completion()),
    });
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp, client.clone()).await;
    let workflow_root = f.workflow_root.clone();
    let spec = DagSpec {
        max_concurrency: 4,
        name: "e2e-sibling".into(),
        description: None,
        steps: vec![agent_step("fast", &[], None), agent_step("slow", &[], None)],
    };
    let run = claimed(spec);
    let run_id = run.run_id.clone();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let run_task = tokio::spawn(async move {
        execute_run(
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
        .unwrap()
    });

    // fast completes; its step_done frame reaches the stub within a batch
    // window (record_step already folded it into `states` by then).
    await_event(&shared, "step_done", Some("fast")).await;
    // Give a (buggy) re-dispatch of slow room to happen and surface.
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let c = shared.lock().unwrap();
        assert_eq!(
            count_events(&c, "step_started", "slow"),
            1,
            "slow was re-dispatched while still in flight"
        );
    }

    // Release slow. The mock's released hang yields an EMPTY stream, which
    // the real session runner folds to `Err("stream ended without
    // completion")` → step Error → run Error. That is orthogonal to the
    // re-dispatch bug this test pins; what matters is that slow converged
    // exactly once and the canary script was never consumed.
    hold.notify_one();
    let status = run_task.await.unwrap();
    assert_eq!(status, DagRunStatus::Error);
    let report = await_status(&shared).await;
    assert_eq!(report.status, "error");
    assert!(report.error.unwrap().contains("step slow"));

    let c = shared.lock().unwrap();
    assert_eq!(count_events(&c, "step_started", "fast"), 1);
    assert_eq!(count_events(&c, "step_done", "fast"), 1);
    assert_eq!(count_events(&c, "step_started", "slow"), 1);
    assert_eq!(count_events(&c, "step_done", "slow"), 1);
    let slow_done = c
        .events
        .iter()
        .find(|e| e.kind == "step_done" && e.step.as_deref() == Some("slow"))
        .unwrap();
    assert_eq!(slow_done.payload["ok"], json!(false));
    drop(c);
    // Exactly one LLM call per step — the canary script stayed unconsumed.
    assert_eq!(mock.fast.call_count(), 1);
    assert_eq!(mock.slow.call_count(), 1);
    // Both steps went through record_step: artifacts are on disk.
    for step in ["fast", "slow"] {
        assert!(
            workflow_root
                .join(&run_id)
                .join(step)
                .join("meta.json")
                .is_file(),
            "missing artifacts for {step}"
        );
    }
}

/// Regression (cancel drain loses in-flight steps): cancelling a run must
/// drain in-flight steps through the same `record_step` path as the main
/// loop — artifacts on disk plus a `step_done` frame upstream — instead of
/// a state-only fold that drops both.
#[tokio::test]
async fn cancel_drain_persists_inflight_step_artifacts_and_frames() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let text = "结论\n```json\n{\"answer\": 1}\n```";
    let hold = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![
                LlmEvent::TextDelta(text.into()),
                LlmEvent::Completed {
                    text: text.into(),
                    tool_calls: vec![],
                    usage: None,
                },
            ]) // fast: completes immediately
            .push_hang(Arc::clone(&hold)), // slow: parked until released
    );
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp, client.clone()).await;
    let workflow_root = f.workflow_root.clone();
    // slow runs after fast so the captured frame order pins the schedule.
    let spec = DagSpec {
        max_concurrency: 4,
        name: "e2e-cancel-drain".into(),
        description: None,
        steps: vec![
            agent_step("fast", &[], None),
            agent_step("slow", &["fast"], None),
        ],
    };
    let run = claimed(spec);
    let run_id = run.run_id.clone();
    let (tx, cancel_rx) = tokio::sync::watch::channel(false);
    let run_task = tokio::spawn(async move {
        execute_run(
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
        .unwrap()
    });

    // fast is recorded; slow is now the only in-flight step (parked).
    await_event(&shared, "step_done", Some("fast")).await;
    tx.send(true).unwrap();
    // Let the loop head observe the flip and enter the drain — which parks
    // joining slow — before releasing it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    hold.notify_one(); // empty stream ends → slow folds to Cancelled

    let status = run_task.await.unwrap();
    assert_eq!(status, DagRunStatus::Cancelled);
    let report = await_status(&shared).await;
    assert_eq!(report.status, "cancelled");

    let c = shared.lock().unwrap();
    let order: Vec<(&str, Option<&str>)> = c
        .events
        .iter()
        .filter(|e| e.kind != "step_log")
        .map(|e| (e.kind.as_str(), e.step.as_deref()))
        .collect();
    assert_eq!(
        order,
        vec![
            ("run_started", None),
            ("step_started", Some("fast")),
            ("step_done", Some("fast")),
            ("step_started", Some("slow")),
            ("step_done", Some("slow")),
            ("run_finished", None),
        ]
    );
    let slow_done = c
        .events
        .iter()
        .find(|e| e.kind == "step_done" && e.step.as_deref() == Some("slow"))
        .unwrap();
    assert_eq!(slow_done.payload["ok"], json!(false));
    assert_eq!(
        c.events.last().unwrap().payload["status"],
        json!("cancelled")
    );
    drop(c);

    // slow went through record_step inside the cancel drain: its artifacts
    // landed on disk (a state-only drain leaves this file missing).
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(workflow_root.join(&run_id).join("slow").join("meta.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(meta["outcome"], json!("cancelled"));
}
