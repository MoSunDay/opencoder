use super::*;

/// Concurrency gate: with `max_concurrency: 2` and four independent agent
/// steps, only two may run at once — the third `step_started` frame must
/// arrive after some `step_done` frame freed a slot. The instant mock makes
/// completion order racy, but the gate invariant is not.
#[tokio::test]
async fn max_concurrency_gates_simultaneous_step_starts() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let text = "结论\n```json\n{\"answer\": 1}\n```";
    let mock = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp).await;
    let spec = DagSpec {
        max_concurrency: 2,
        name: "e2e-capped".into(),
        description: None,
        steps: vec![
            agent_step("a", &[], None),
            agent_step("b", &[], None),
            agent_step("c", &[], None),
            agent_step("d", &[], None),
        ],
    };
    let run = claimed(spec);
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&f.uplink),
            exec: ExecDeps {
                store: Arc::clone(&f.store),
                client,
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
    await_status(&shared).await;
    // The terminal status is posted after the event flush: every frame is
    // captured by now, in emit order.
    let frames = {
        let c = shared.lock().unwrap();
        kinds(&c)
    };
    // Sanity: every step started and finished exactly once; 4 chat calls.
    {
        let c = shared.lock().unwrap();
        for name in ["a", "b", "c", "d"] {
            assert_eq!(count_events(&c, "step_started", name), 1, "{name}");
            assert_eq!(count_events(&c, "step_done", name), 1, "{name}");
        }
    }
    assert_eq!(mock.call_count(), 4);

    // Index positions of the start/done frames in arrival order.
    let idx = |kind: &str, n: usize| {
        frames
            .iter()
            .enumerate()
            .filter(|(_, k)| k.as_str() == kind)
            .nth(n)
            .map(|(i, _)| i)
            .unwrap_or_else(|| panic!("missing {kind} #{n}: {frames:?}"))
    };
    let (third_start, first_done) = (idx("step_started", 2), idx("step_done", 0));
    assert!(
        first_done < third_start,
        "third step started before a slot freed: {frames:?}"
    );
}

/// `max_concurrency: 1` serializes the whole run: each `step_started` lands
/// only after the previous `step_done` (no overlap at all).
#[tokio::test]
async fn max_concurrency_one_runs_strictly_serial() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let text = "结论\n```json\n{\"answer\": 1}\n```";
    let mock = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp).await;
    let spec = DagSpec {
        max_concurrency: 1,
        name: "e2e-serial".into(),
        description: None,
        steps: vec![
            agent_step("a", &[], None),
            agent_step("b", &[], None),
            agent_step("c", &[], None),
        ],
    };
    let run = claimed(spec);
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&f.uplink),
            exec: ExecDeps {
                store: Arc::clone(&f.store),
                client,
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
    await_status(&shared).await;
    let c = shared.lock().unwrap();
    let serial: Vec<&str> = vec![
        "run_started",
        "step_started",
        "step_done",
        "step_started",
        "step_done",
        "step_started",
        "step_done",
        "run_finished",
    ];
    assert_eq!(kinds(&c), serial, "steps overlapped: {:?}", kinds(&c));
}
