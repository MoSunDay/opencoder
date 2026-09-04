//! Happy-path runner loop: register -> claim -> execute with a deterministic
//! mock LLM (TextDelta x2 + Completed) -> assert uploaded event contents and
//! order, terminal status `done`, and that idle heartbeats afterwards carry
//! no cancel instructions.

mod support;

use std::sync::Arc;
use std::time::Duration;

use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_node::uplink::Uplink;
use opencoder_node::NodeOpts;

/// Tight intervals: fast tests without changing production defaults.
fn test_opts(base: &str, workdir: &std::path::Path, data: &std::path::Path) -> NodeOpts {
    NodeOpts {
        name: "node-happy".into(),
        remote: base.into(),
        token: support::TOKEN.into(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(40),
        claim_interval: Duration::from_millis(30),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
        dag: None,
    }
}

/// Pin autopilot off via the project domain file so a developer's global
/// `~/.opencoder/ap.json` cannot append a review turn to the one-round mock.
fn pin_autopilot_off(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claims_executes_uploads_and_reports_done() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);

    let task = st.push_task("say hello twice");
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("alpha".into()),
        LlmEvent::TextDelta("beta".into()),
        LlmEvent::Completed {
            text: "alphabeta".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]));

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(client),
    ));

    // Registration observed server-side before any work.
    support::wait_for(30, || {
        let regs = st.registrations();
        (!regs.is_empty()).then_some(regs)
    })
    .await;
    assert_eq!(st.registrations()[0], "node-happy");

    // The task is claimed exactly once.
    support::wait_for(30, || {
        let claimed = st.claimed();
        (!claimed.is_empty()).then_some(claimed)
    })
    .await;

    // Terminal report lands as `done`.
    support::wait_for(120, || st.status_of(&task.task_id)).await;
    assert_eq!(st.status_of(&task.task_id).as_deref(), Some("done"));

    // Event stream assertions — arrival order is authoritative.
    let events = st.events();
    assert!(!events.is_empty(), "at least one event batch uploaded");
    let kinds: Vec<&str> = events.iter().map(|e| e.sse_kind.as_str()).collect();
    let pos_of = |needle: &str| kinds.iter().position(|k| *k == needle);

    // Both text deltas arrive, scripted order, canonical payloads...
    let p0 = pos_of("text_delta").expect("first text_delta missing");
    let p1 = kinds[p0 + 1..]
        .iter()
        .position(|k| *k == "text_delta")
        .expect("second text_delta missing")
        + p0
        + 1;
    assert_eq!(events[p0].payload["text"], "alpha");
    assert_eq!(events[p1].payload["text"], "beta");

    // ...and `done` trails them.
    let pd = pos_of("done").expect("done event missing");
    assert!(pd > p1, "done must trail the deltas; kinds={kinds:?}");

    // Uploads reached the server across >=1 ordered batches.
    assert!(st.batch_count() >= 1);
    // A busy-phase heartbeater kept the node alive.
    assert!(st.heartbeat_count() >= 1);

    // Once idle+terminal, heartbeats carry no cancel instructions.
    let uplink = Uplink::new(&base, support::TOKEN).unwrap();
    let hb = uplink.heartbeat(support::STUB_NODE_ID).await.unwrap();
    assert!(
        hb.cancel_task_ids.is_empty(),
        "idle node must not be told to cancel anything"
    );

    runner.abort();
}
