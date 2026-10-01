use super::*;

#[tokio::test]
async fn single_agent_step_completes_and_reports_done() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let text = "结论如下\n```json\n{\"answer\": 42}\n```";
    let mock = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp, client.clone()).await;
    let run = claimed(one_step_spec());
    let run_id = run.run_id.clone();
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

    assert_eq!(status, DagRunStatus::Done);
    let report = await_status(&shared).await;
    assert_eq!(report.status, "done");
    assert!(report.error.is_none());

    // Scope the capture guard: the artifact assertions below await the store.
    {
        let c = shared.lock().unwrap();
        assert_eq!(
            kinds(&c),
            vec!["run_started", "step_started", "step_done", "run_finished"]
        );
        assert!(c.events.iter().any(|e| e.kind == "step_log"
            && e.step.as_deref() == Some("analyze")
            && e.payload["event"] == "text_delta"
            && e.payload["data"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("answer"))));
        let step_done = c.events.iter().find(|e| e.kind == "step_done").unwrap();
        assert_eq!(step_done.step.as_deref(), Some("analyze"));
        assert_eq!(step_done.payload["ok"], json!(true));
        assert_eq!(c.events.last().unwrap().payload["status"], json!("done"));
    }

    // Artifacts under <workflow_root>/<run_id>/<step>/.
    let dir = f.workflow_root.join(&run_id).join("analyze");
    let written = std::fs::read_to_string(dir.join("output.txt")).unwrap();
    assert!(written.contains("answer"));
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("output.json")).unwrap()).unwrap();
    assert_eq!(parsed, json!({"answer": 42}));
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["outcome"], json!("done"));

    // `session.json` is the LIVE pointer, written when the session was
    // created: it must resolve to a real store session and agree with the
    // `session_id` meta.json carries for finished steps (LOCKED contract).
    let session_file = opencoder_dag::artifacts::session_file(&dir);
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_file).unwrap()).unwrap();
    let live = live_session_id(&session);
    assert_eq!(session["agent"], "act");
    assert_eq!(session["harness"], "opencoder");
    assert_eq!(session["status"], "done");
    assert_eq!(meta["session_id"], json!(live));
    let store: &dyn opencoder_store::Store = f.store.as_ref();
    assert!(store.get_session(&live).await.unwrap().is_some());

    // The step ran on a real (mocked-LLM) session: exactly one chat call.
    assert_eq!(mock.call_count(), 1);
}

/// The session id carried by a parsed `session.json`.
fn live_session_id(value: &serde_json::Value) -> String {
    opencoder_dag::artifacts::parse_session_id(value).expect("session.json carries a session_id")
}

const HELLO_C: &str =
    "#include <stdio.h>\nint main(void) { puts(\"from native step\"); return 0; }";

#[tokio::test]
async fn native_step_output_is_mirrored_to_the_run_session() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let client: Arc<dyn opencoder_llm::ChatStream> = Arc::new(MockChatClient::new());
    let f = fixture(&base, &tmp, client.clone()).await;
    let spec = DagSpec {
        max_concurrency: 4,
        name: "e2e-native".into(),
        description: None,
        steps: vec![StepSpec {
            trigger_rule: Default::default(),
            name: "build".into(),
            depends_on: vec![],
            kind: StepKind::Binary {
                resource: "tool".into(),
                args: vec![],
            },
            timeout_secs: None,
        }],
    };
    let run = claimed(spec);
    let run_id = run.run_id.clone();
    // Stage the module in the run's context root, as a spec upload would.
    let run_root = f.workflow_root.join(&run_id);
    std::fs::create_dir_all(&run_root).unwrap();
    let source = tmp.path().join("tool.c");
    let binary = tmp.path().join("tool");
    std::fs::write(&source, HELLO_C).unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-O2", "-static"])
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .status()
        .unwrap()
        .success());
    opencoder_dag_binary::save_binary_version(
        &f.container.pool,
        "tool",
        "fixture",
        &std::fs::read(binary).unwrap(),
    )
    .unwrap();
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
    assert_eq!(status, DagRunStatus::Done);
    assert_eq!(await_status(&shared).await.status, "done");

    let store: &dyn opencoder_store::Store = f.store.as_ref();
    let rows = store.events_after(&run_id, 0).await.unwrap();
    let mirrored: Vec<_> = rows
        .iter()
        .filter(|row| row.sse_kind.as_deref() == Some("step_output"))
        .collect();
    assert_eq!(mirrored.len(), 1, "{rows:?}");
    assert_eq!(mirrored[0].session_id, run_id);
    assert_eq!(mirrored[0].payload["step"], json!("build"));
    assert_eq!(mirrored[0].payload["stream"], json!("stdout"));
    assert_eq!(mirrored[0].payload["text"], json!("from native step\n"));
    assert_eq!(
        mirrored[0].payload["at_ms"].as_i64().unwrap(),
        mirrored[0].ts
    );

    // Artifacts are unchanged, and a native step reports no sub-session.
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_root.join("build").join("meta.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["outcome"], json!("done"));
    assert!(meta["session_id"].is_null());

    // Single-writer guard: native output has exactly ONE writer — the node-store
    // `step_output` mirror (`src/step_log.rs`). `step_log` (run-level Uplink
    // report, mirrored back onto the run session) only carries an agent step's
    // `text_delta`, so the two chains are mutually exclusive per step kind and
    // neither the run-wide log nor a single-step record can show duplicate rows.
    assert_eq!(
        rows.iter()
            .filter(|row| row.sse_kind.as_deref() == Some("step_log"))
            .count(),
        0,
        "{rows:?}"
    );
    let c = shared.lock().unwrap();
    assert_eq!(count_events(&c, "step_log", "build"), 0, "{rows:?}");
}

#[tokio::test]
async fn failed_upstream_blocks_dependent_and_folds_run_error() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    // The LLM stream never yields: the 1s step budget fires, the step folds
    // to Error, and the dependent step is transitively blocked.
    let hold = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(MockChatClient::new().push_hang(Arc::clone(&hold)));
    let f = fixture(&base, &tmp, client.clone()).await;

    let spec = DagSpec {
        max_concurrency: 4,
        name: "e2e-chain".into(),
        description: None,
        steps: vec![
            agent_step("analyze", &[], Some(1)),
            agent_step("report", &["analyze"], None),
        ],
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
    assert!(report.error.unwrap().contains("analyze"));

    let c = shared.lock().unwrap();
    let blocked = c
        .events
        .iter()
        .find(|e| e.step.as_deref() == Some("report"))
        .expect("blocked step emits a step_done frame");
    assert_eq!(blocked.kind, "step_done");
    assert_eq!(blocked.payload["ok"], json!(false));
    assert!(blocked.payload["error"]
        .as_str()
        .unwrap()
        .contains("blocked"));
    let finished = c.events.last().unwrap();
    assert_eq!(finished.kind, "run_finished");
    assert_eq!(finished.payload["status"], json!("error"));
}
