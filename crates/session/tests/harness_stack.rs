#![cfg(unix)]
#[path = "harness/fixtures.rs"]
mod fixtures;

#[test]
fn codex_operator_runs_and_resumes_on_a_normal_worker_stack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::spawn(async {
            let root = tempfile::tempdir().unwrap();
            let resources = tempfile::tempdir().unwrap();
            fixtures::card(resources.path());
            let (mut session, store) = fixtures::session(root.path()).await;
            session.config.agent.agents_dir = Some(resources.path().into());
            store
                .create_session(&opencoder_store::SessionMeta {
                    id: session.id.clone(),
                    kind: Some("operator".into()),
                    agent: Some("custom".into()),
                    ..Default::default()
                })
                .await
                .unwrap();
            store
                .set_harness_runtime(&session.id, &session.harness)
                .await
                .unwrap();
            for prompt in ["first turn", "resume turn"] {
                let mut restored = opencoder_session::resume(
                    store.clone(),
                    &session.id,
                    session.config.clone(),
                    session.client.clone(),
                    root.path().into(),
                )
                .await
                .unwrap();
                opencoder_session::run(&mut restored, prompt.into(), |_| {})
                    .await
                    .unwrap();
            }
            let captured = std::fs::read_to_string(root.path().join("capture.jsonl")).unwrap();
            let records: Vec<serde_json::Value> = captured
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(records.len(), 2);
            assert_eq!(records[1]["args"][1], "resume");
        })
        .await
        .unwrap();
    });
}
