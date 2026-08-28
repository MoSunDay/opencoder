//! P3 control-plane relay on the worker side: a control task delivered by the
//! stub server (claim reply while idle, heartbeat batch while busy) must be
//! served from the node's LOCAL store and uploaded via `control_result`.
//!
//! - `claim_delivers_control_and_worker_uploads_slice`: run one real task
//!   (mock LLM) so the local store holds the transcript, then a
//!   `fetch_messages` control rides the claim reply -> the uploaded result is
//!   `ok:true` with a resume-shaped slice of that dialog.
//! - `unknown_session_reports_ok_false`: a control for a dialog the node
//!   never saw reports the miss instead of timing out the browser.
//! - `heartbeat_delivers_control_while_busy`: a control arriving while the
//!   worker is mid-task (LLM call parked) is still picked up — via the busy
//!   heartbeater — proving the "busy nodes never poll claim" delivery path.

mod support;

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::node_protocol::{ControlTask, TASK_KIND_FETCH_MESSAGES};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_node::NodeOpts;
use opencoder_store::Store;

fn test_opts(base: &str, workdir: &std::path::Path, data: &std::path::Path) -> NodeOpts {
    NodeOpts {
        name: "node-control".into(),
        remote: base.into(),
        token: support::TOKEN.into(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(50),
        claim_interval: Duration::from_millis(30),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
    }
}

fn pin_autopilot_off(workdir: &std::path::Path) {
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();
}

fn mock_round() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("alpha".into()),
        LlmEvent::Completed {
            text: "alpha".into(),
            tool_calls: vec![],
            usage: None,
        },
    ]))
}

fn control(control_id: &str, session_id: &str) -> ControlTask {
    ControlTask {
        control_id: control_id.into(),
        kind: TASK_KIND_FETCH_MESSAGES.into(),
        session_id: session_id.into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claim_delivers_control_and_worker_uploads_slice() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);

    // One executed task => the node's LOCAL store now owns the dialog.
    let task = st.push_task("relay me");
    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(mock_round()),
    ));
    support::wait_for(120, || st.status_of(&task.task_id)).await;
    assert_eq!(st.status_of(&task.task_id).as_deref(), Some("done"));

    // Control delivered through the claim reply (node is idle again).
    st.push_control(control("ctl-idle", &task.session_id));
    let result = support::wait_for(120, || {
        st.control_results()
            .into_iter()
            .find(|r| r.control_id == "ctl-idle")
    })
    .await;
    assert!(result.ok, "error={:?}", result.error);
    assert_eq!(result.session_id, task.session_id);
    assert!(
        result.summary.is_none() && result.summary_seq.is_none(),
        "uncompacted dialog has no summary pair"
    );
    // Resume-shaped slice: the user prompt first, then the assistant turn.
    assert!(result.messages.len() >= 2, "messages={:?}", result.messages);
    assert_eq!(result.messages[0].role, "user");
    assert_eq!(result.messages[0].seq, 1);
    let json = serde_json::to_value(&result.messages[0].blocks).unwrap();
    assert_eq!(json[0]["kind"], "text");
    assert!(result.messages.iter().any(|m| m.role == "assistant"));
    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_session_reports_ok_false() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(mock_round()),
    ));
    st.push_control(control("ctl-ghost", "sess-never-existed"));
    let result = support::wait_for(120, || {
        st.control_results()
            .into_iter()
            .find(|r| r.control_id == "ctl-ghost")
    })
    .await;
    assert!(!result.ok, "a missing dialog must report failure");
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("not found"),
        "error={:?}",
        result.error
    );
    assert!(result.messages.is_empty());
    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_delivers_control_while_busy() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    pin_autopilot_off(&workdir);

    // Park the LLM call so the worker is BUSY (no claim polling at all).
    let task = st.push_task("hang while a control arrives");
    let notify = Arc::new(tokio::sync::Notify::new());
    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(&base, &workdir, data.path()),
        Some(Arc::new(MockChatClient::new().push_hang(notify.clone()))),
    ));
    support::wait_for(30, || st.claimed().contains(&task.task_id).then_some(())).await;

    // Wait until the prompt is DURABLE in the node's local store before
    // pushing the control: the runner persists the user message just before
    // the LLM call parks, and under heavy test-machine load the busy
    // heartbeater could otherwise serve the fetch slice inside that startup
    // window, racing the assertion below on an empty transcript.
    let local = opencoder_store::LibsqlStore::open(data.path().join("opencoder.db"))
        .await
        .unwrap();
    let sid = task.session_id.clone();
    let durable = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let has_user_msg = local
            .load_messages(&sid)
            .await
            .unwrap_or_default()
            .iter()
            .any(|m| m.role == opencoder_core::Role::User);
        if has_user_msg {
            break;
        }
        assert!(
            std::time::Instant::now() < durable,
            "user prompt never became durable in the node's local store"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Control arrives while busy: only the heartbeat can deliver it.
    st.push_control(control("ctl-busy", &task.session_id));
    let result = support::wait_for(120, || {
        st.control_results()
            .into_iter()
            .find(|r| r.control_id == "ctl-busy")
    })
    .await;
    assert_eq!(result.control_id, "ctl-busy");
    assert_eq!(result.session_id, task.session_id);
    // The user prompt is persisted before the LLM call hangs, so the slice is
    // already meaningful even mid-run.
    assert!(result.ok, "error={:?}", result.error);
    assert!(
        result.messages.iter().any(|m| m.role == "user"),
        "messages={:?}",
        result.messages
    );
    runner.abort();
    notify.notify_waiters();
}
