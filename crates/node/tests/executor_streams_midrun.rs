//! Regression: a claimed task's event batches must reach the server WHILE
//! the task runs, not sit in the local unbounded channel until the terminal
//! state (long runs would pile memory up unboundedly and the server-side SSE
//! stream would stay blank the whole time). The LLM backend here refuses to
//! finish the run until the stub server has received at least one uploaded
//! event, so a run-end-deferred uploader deterministically fails this test.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use opencoder_core::node_protocol::ClaimedTask;
use opencoder_core::Config;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_node::executor::{execute, ExecDeps};
use opencoder_node::uplink::Uplink;
use opencoder_store::LibsqlStore;
use tokio::sync::mpsc;

/// LLM backend whose script is: delta -> window-sized gap -> delta -> wait
/// for a server-side upload -> Completed. The gap (> `batcher::WINDOW`,
/// 300ms) makes the second callback push observe an expired window and flush
/// the buffered batch; the gate then only releases once that batch has been
/// uploaded. If the uploader only ran after the drain, the gate would stall
/// past its deadline and `released_midrun` stays false.
struct UploadGateClient {
    stub: Arc<support::Stub>,
    released_midrun: Arc<AtomicBool>,
}

impl ChatStream for UploadGateClient {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel(8);
        let stub = Arc::clone(&self.stub);
        let released_midrun = Arc::clone(&self.released_midrun);
        tokio::spawn(async move {
            let _ = tx.send(LlmEvent::TextDelta("warm".into())).await;
            // Larger than batcher::WINDOW: the next push flushes the batch.
            tokio::time::sleep(Duration::from_millis(350)).await;
            let _ = tx.send(LlmEvent::TextDelta("late".into())).await;
            // Release only after the server has ACTUALLY received an event;
            // the deadline keeps a broken pipeline from hanging the test.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while stub.events().is_empty() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "uploader never delivered a batch while the run was in flight"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            released_midrun.store(true, Ordering::SeqCst);
            let _ = tx
                .send(LlmEvent::Completed {
                    text: "gated".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(rx)
    }

    fn backend(&self) -> &'static str {
        "upload-gate-mock"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_upload_while_task_still_running() {
    let (base, st) = support::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    // Pin autopilot off so the one-round mock is not extended by a review
    // turn (same hygiene as runner_happy).
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(
        workdir.join(".opencoder").join("ap.json"),
        r#"{"mode":"off"}"#,
    )
    .unwrap();

    let released_midrun = Arc::new(AtomicBool::new(false));
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open_memory().await.unwrap());
    let client: Arc<dyn ChatStream> = Arc::new(UploadGateClient {
        stub: Arc::clone(&st),
        released_midrun: Arc::clone(&released_midrun),
    });
    let task = ClaimedTask {
        task_id: "task-stream-1".into(),
        session_id: "sess-stream-1".into(),
        title: None,
        prompt: "stream events while running".into(),
        agent: Some("act".into()),
        model: Some("m/g".into()),
        created_at: 0,
    };
    let deps = ExecDeps {
        store,
        client,
        workdir: workdir.clone(),
        config: Config {
            model: "m/g".into(),
            ..Default::default()
        },
    };
    let uplink = Uplink::new(&base, support::TOKEN).unwrap();
    let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    execute(&uplink, deps, &task, cancel_rx).await.unwrap();

    assert!(
        released_midrun.load(Ordering::SeqCst),
        "the run finished before the server saw any event: uploads were \
         deferred past the drain instead of streaming concurrently"
    );
    assert!(!st.events().is_empty(), "server must have received events");
    assert_eq!(
        st.status_of(&task.task_id).as_deref(),
        Some("done"),
        "terminal report must still land last"
    );
}
