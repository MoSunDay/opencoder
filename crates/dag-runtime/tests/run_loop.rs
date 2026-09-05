//! Integration tests for the whole-run scheduling loop (`execute_run`):
//! a local axum stub stands in for the server's DAG uplink endpoints and
//! captures every event batch + terminal status report, while the agent
//! step runs on the real session runner with a `MockChatClient`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use opencoder_dag::protocol::{DagClaimedRun, DagEventBatch, DagStatusReport};
use opencoder_dag::{DagEventIn, DagRunStatus, DagSpec, StepKind, StepSpec};
use opencoder_dag_runtime::{execute_run, ExecDeps, RunDeps};
use opencoder_llm::event::LlmEvent;
use opencoder_llm::MockChatClient;
use opencoder_node::uplink::Uplink;
use opencoder_store::LibsqlStore;
use serde_json::json;
use tokio::net::TcpListener;

/// Everything the stub server captured, in arrival order.
#[derive(Default)]
struct Captured {
    events: Vec<DagEventIn>,
    statuses: Vec<DagStatusReport>,
}

type Shared = Arc<Mutex<Captured>>;

/// Spin up the two uplink endpoints (`events` + `status`) on an ephemeral
/// port; returns the base URL and the capture handle.
async fn spawn_stub() -> (String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Captured::default()));
    let app = Router::new()
        .route(
            "/api/nodes/dag/runs/:rid/events",
            post(
                |State(s): State<Shared>, Json(batch): Json<DagEventBatch>| async move {
                    let mut c = s.lock().unwrap();
                    c.events.extend(batch.events);
                    StatusCode::OK
                },
            ),
        )
        .route(
            "/api/nodes/dag/runs/:rid/status",
            post(
                |State(s): State<Shared>,
                 AxPath(_rid): AxPath<String>,
                 Json(report): Json<DagStatusReport>| async move {
                    s.lock().unwrap().statuses.push(report);
                    StatusCode::OK
                },
            ),
        )
        .with_state(Arc::clone(&shared));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), shared)
}

/// One-step agent spec whose transcript ends in a ```json fence.
fn one_step_spec() -> DagSpec {
    DagSpec {
        name: "e2e-one".into(),
        description: None,
        steps: vec![StepSpec {
            name: "analyze".into(),
            depends_on: vec![],
            kind: StepKind::Agent {
                prompt: "给出结论".into(),
                agent: None,
                model: None,
            },
            timeout_secs: None,
        }],
    }
}

fn agent_step(name: &str, deps: &[&str], timeout: Option<u64>) -> StepSpec {
    StepSpec {
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        kind: StepKind::Agent {
            prompt: format!("{name} prompt"),
            agent: None,
            model: None,
        },
        timeout_secs: timeout,
    }
}

fn claimed(spec: DagSpec) -> DagClaimedRun {
    DagClaimedRun {
        run_id: ulid::Ulid::new().to_string(),
        dag_id: ulid::Ulid::new().to_string(),
        spec,
        created_at: 0,
    }
}

fn kinds(c: &Captured) -> Vec<String> {
    c.events.iter().map(|e| e.kind.clone()).collect()
}

/// Wait until the stub saw at least one status report (the loop posts it
/// only after the event flush, so it is the natural convergence point).
async fn await_status(shared: &Shared) -> DagStatusReport {
    for _ in 0..250 {
        if let Some(r) = shared.lock().unwrap().statuses.last() {
            return r.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no status report arrived within 5s");
}

/// Poll the stub until one `kind` event for `step` arrives (5s cap) — the
/// event uploader flushes on count cap or 300ms window, so arrival is
/// asynchronous to the scheduling loop that emitted it.
async fn await_event(shared: &Shared, kind: &str, step: Option<&str>) -> DagEventIn {
    for _ in 0..250 {
        {
            let c = shared.lock().unwrap();
            if let Some(ev) = c
                .events
                .iter()
                .find(|e| e.kind == kind && e.step.as_deref() == step)
            {
                return ev.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no {kind} event for step {step:?} arrived within 5s");
}

/// How many `kind` events the stub captured for one step.
fn count_events(c: &Captured, kind: &str, step: &str) -> usize {
    c.events
        .iter()
        .filter(|e| e.kind == kind && e.step.as_deref() == Some(step))
        .count()
}

/// Per-test runtime inputs: uplink against the stub, a real LibsqlStore in
/// the temp dir, and the default Config for that workdir.
struct Fixture {
    uplink: Arc<Uplink>,
    workdir: PathBuf,
    workflow_root: PathBuf,
    store: Arc<dyn opencoder_store::Store>,
    config: opencoder_core::Config,
}

async fn fixture(base: &str, tmp: &tempfile::TempDir) -> Fixture {
    let workdir = tmp.path().to_path_buf();
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open(workdir.join("store.db")).await.unwrap());
    let config = opencoder_core::Config::load(&workdir).unwrap();
    Fixture {
        uplink: Arc::new(Uplink::new(base, "test-token").unwrap()),
        workdir: workdir.clone(),
        workflow_root: tmp.path().join("workflow"),
        store,
        config,
    }
}

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
    let f = fixture(&base, &tmp).await;
    let run = claimed(one_step_spec());
    let run_id = run.run_id.clone();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);

    let status = execute_run(
        RunDeps {
            uplink: Arc::clone(&f.uplink),
            exec: ExecDeps {
                store: Arc::clone(&f.store),
                client: Arc::clone(&client),
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

    let c = shared.lock().unwrap();
    assert_eq!(
        kinds(&c),
        vec!["run_started", "step_started", "step_done", "run_finished"]
    );
    let step_done = &c.events[2];
    assert_eq!(step_done.step.as_deref(), Some("analyze"));
    assert_eq!(step_done.payload["ok"], json!(true));
    assert_eq!(c.events[3].payload["status"], json!("done"));
    drop(c);

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
    // The step ran on a real (mocked-LLM) session: exactly one chat call.
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn failed_upstream_blocks_dependent_and_folds_run_error() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    // The LLM stream never yields: the 1s step budget fires, the step folds
    // to Error, and the dependent step is transitively blocked.
    let hold = Arc::new(tokio::sync::Notify::new());
    let client = Arc::new(MockChatClient::new().push_hang(Arc::clone(&hold)));
    let f = fixture(&base, &tmp).await;

    let spec = DagSpec {
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

#[tokio::test]
async fn invalid_spec_snapshot_fails_before_scheduling() {
    let (base, shared) = spawn_stub().await;
    let tmp = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new());
    let f = fixture(&base, &tmp).await;

    // Cycle: never dispatchable, but constructible in-memory — the runtime
    // must fold it into a clean error report instead of wedging.
    let spec = DagSpec {
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
    let f = fixture(&base, &tmp).await;

    let run = claimed(one_step_spec());
    let (tx, cancel_rx) = tokio::sync::watch::channel(false);
    tx.send(true).unwrap();
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
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(completion()) // fast: completes immediately
            .push_hang(Arc::clone(&hold)) // slow: parked until released
            .push_script(completion()), // canary: must stay unconsumed
    );
    let client: Arc<dyn opencoder_llm::ChatStream> = mock.clone();
    let f = fixture(&base, &tmp).await;
    let workflow_root = f.workflow_root.clone();
    // Spec order = spawn order = mock script consumption order.
    let spec = DagSpec {
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
    assert_eq!(mock.call_count(), 2);
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
    let f = fixture(&base, &tmp).await;
    let workflow_root = f.workflow_root.clone();
    // slow runs after fast so the captured frame order pins the schedule.
    let spec = DagSpec {
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
