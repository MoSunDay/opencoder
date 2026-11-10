//! Runner-loop tests for the DAG claim arm: with a `DagHook` injected, an
//! idle node (no prompt tasks due) polls `/api/nodes/dag/claim`, executes
//! the claimed run through the hook under its own heartbeater, and a
//! heartbeat `cancel_run_ids` flip converges into the hook's cancel flag.
//! The stub now lives in `support/dag.rs`, same wire-contract style as
//! `support/`; the fake hook's `claim` goes through the real
//! Bearer-authenticated uplink (200/204 exercised).

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use opencoder_dag::protocol::DagClaimedRun;
use opencoder_dag::{DagSpec, StepKind, StepSpec};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_node::uplink::Uplink;
use opencoder_node::{DagHook, NodeOpts};
use tokio::sync::{watch, Notify};

use support::dag;

/// Fake executor: `claim` uses the real Bearer-authenticated uplink; `execute` records the
/// run and (optionally) parks until the runner flips the cancel flag.
struct FakeHook {
    uplink: Uplink,
    claim_node_ids: Mutex<Vec<String>>,
    executed: Mutex<Vec<String>>,
    started: Arc<Notify>,
    finished: Arc<Notify>,
    park_for_cancel: bool,
    observed_cancel: AtomicBool,
}

#[async_trait]
impl DagHook for FakeHook {
    async fn claim(&self, node_id: &str) -> anyhow::Result<Option<DagClaimedRun>> {
        self.claim_node_ids
            .lock()
            .unwrap()
            .push(node_id.to_string());
        self.uplink.dag_claim(node_id).await
    }

    async fn execute(
        &self,
        run: DagClaimedRun,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        self.executed.lock().unwrap().push(run.run_id.clone());
        self.started.notify_one();
        if self.park_for_cancel {
            while !*cancel_rx.borrow_and_update() {
                if cancel_rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            self.observed_cancel.store(true, Ordering::SeqCst);
        }
        self.finished.notify_one();
        Ok(())
    }
}

fn queued_run(run_id: &str) -> DagClaimedRun {
    DagClaimedRun {
        run_id: run_id.to_string(),
        dag_id: "dag-1".to_string(),
        spec: DagSpec {
            max_concurrency: 4,
            name: "stub-flow".into(),
            description: None,
            steps: vec![StepSpec {
                trigger_rule: Default::default(),
                name: "only".into(),
                depends_on: vec![],
                kind: StepKind::Agent {
                    prompt: "p".into(),
                    agent: None,
                    model: None,
                    how_append: None,
                },
                timeout_secs: None,
                recovery_policy: Default::default(),
            }],
        },
        created_at: 0,
    }
}

fn test_opts(
    base: &str,
    workdir: &std::path::Path,
    data: &std::path::Path,
    dag: Arc<dyn DagHook>,
) -> NodeOpts {
    NodeOpts {
        name: "node-dag".into(),
        remote: base.into(),
        token: support::dag::TOKEN.into(),
        workdir: workdir.to_path_buf(),
        heartbeat_interval: Duration::from_millis(40),
        claim_interval: Duration::from_millis(30),
        version: env!("CARGO_PKG_VERSION").into(),
        local_store_dir: Some(data.to_path_buf()),
        dag: Some(dag),
    }
}

fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_loop_claims_dag_run_and_executes_it_via_hook() {
    let (base, st) = dag::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    st.queue_run(queued_run("run-happy"));

    let hook = Arc::new(FakeHook {
        uplink: Uplink::new(&base, dag::TOKEN).unwrap(),
        claim_node_ids: Mutex::new(vec![]),
        executed: Mutex::new(vec![]),
        started: Arc::new(Notify::new()),
        finished: Arc::new(Notify::new()),
        park_for_cancel: false,
        observed_cancel: AtomicBool::new(false),
    });

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(
            &base,
            &workdir,
            data.path(),
            hook.clone() as Arc<dyn DagHook>,
        ),
        Some(mock_client()),
    ));

    // Registration precedes any work; the run is claimed exactly once.
    support::wait_for(30, || (!st.lock().registered.is_empty()).then_some(())).await;
    let ids = support::wait_for(30, || {
        let ids = st.lock().claimed_run_ids.clone();
        (!ids.is_empty()).then_some(ids)
    })
    .await;
    assert_eq!(ids, vec!["run-happy".to_string()]);

    // The hook executed it, claiming under the registered node id.
    support::wait_for(30, || {
        let ex = hook.executed.lock().unwrap().clone();
        (!ex.is_empty()).then_some(ex)
    })
    .await;
    assert_eq!(hook.executed.lock().unwrap().as_slice(), ["run-happy"]);
    assert!(hook
        .claim_node_ids
        .lock()
        .unwrap()
        .iter()
        .all(|id| id == dag::STUB_NODE_ID));
    assert!(!hook.observed_cancel.load(Ordering::SeqCst));

    // The per-run heartbeater kept the node live during execution.
    support::wait_for(30, || (st.heartbeat_count() >= 1).then_some(())).await;
    // The fake executor uploads nothing itself — no stray dag traffic.
    assert!(st.lock().dag_statuses.is_empty());
    // Later claim polls see 204 and the loop stays idle.
    support::wait_for(30, || (st.dag_claims() >= 2).then_some(())).await;

    runner.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_cancel_run_ids_flips_the_hook_cancel_flag() {
    let (base, st) = dag::spawn_stub().await;
    let workdir = tempfile::tempdir().unwrap().keep();
    let data = tempfile::tempdir().unwrap();
    st.queue_run(queued_run("run-stop"));

    let hook = Arc::new(FakeHook {
        uplink: Uplink::new(&base, dag::TOKEN).unwrap(),
        claim_node_ids: Mutex::new(vec![]),
        executed: Mutex::new(vec![]),
        started: Arc::new(Notify::new()),
        finished: Arc::new(Notify::new()),
        park_for_cancel: true,
        observed_cancel: AtomicBool::new(false),
    });

    let runner = tokio::spawn(opencoder_node::run_node(
        test_opts(
            &base,
            &workdir,
            data.path(),
            hook.clone() as Arc<dyn DagHook>,
        ),
        Some(mock_client()),
    ));

    // Wait until the hook is INSIDE execute(), then instruct cancellation.
    support::wait_for(30, || {
        (!hook.executed.lock().unwrap().is_empty()).then_some(())
    })
    .await;
    st.set_cancel_run("run-stop");

    // The parked hook only returns once the flag flipped.
    support::wait_for(30, || {
        hook.observed_cancel.load(Ordering::SeqCst).then_some(())
    })
    .await;
    assert_eq!(
        st.lock().claimed_run_ids.clone(),
        vec!["run-stop".to_string()]
    );

    runner.abort();
}
