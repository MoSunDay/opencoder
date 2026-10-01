use super::*;

/// Everything the stub server captured, in arrival order.
#[derive(Default)]
pub(super) struct Captured {
    pub(super) events: Vec<DagEventIn>,
    pub(super) statuses: Vec<DagStatusReport>,
}

pub(super) type Shared = Arc<Mutex<Captured>>;

/// Spin up the two uplink endpoints (`events` + `status`) on an ephemeral
/// port; returns the base URL and the capture handle.
pub(super) async fn spawn_stub() -> (String, Shared) {
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
pub(super) fn one_step_spec() -> DagSpec {
    DagSpec {
        max_concurrency: 4,
        name: "e2e-one".into(),
        description: None,
        steps: vec![StepSpec {
            trigger_rule: Default::default(),
            name: "analyze".into(),
            depends_on: vec![],
            kind: StepKind::Agent {
                prompt: "给出结论".into(),
                agent: None,
                model: None,
                how_append: None,
            },
            timeout_secs: None,
        }],
    }
}

pub(super) fn agent_step(name: &str, deps: &[&str], timeout: Option<u64>) -> StepSpec {
    StepSpec {
        trigger_rule: Default::default(),
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        kind: StepKind::Agent {
            prompt: format!("{name} prompt"),
            agent: None,
            model: None,
            how_append: None,
        },
        timeout_secs: timeout,
    }
}

pub(super) fn claimed(spec: DagSpec) -> DagClaimedRun {
    DagClaimedRun {
        run_id: ulid::Ulid::new().to_string(),
        dag_id: ulid::Ulid::new().to_string(),
        spec,
        created_at: 0,
    }
}

pub(super) fn kinds(c: &Captured) -> Vec<String> {
    c.events
        .iter()
        .filter(|e| e.kind != "step_log")
        .map(|e| e.kind.clone())
        .collect()
}

/// Wait until the stub saw at least one status report (the loop posts it
/// only after the event flush, so it is the natural convergence point).
pub(super) async fn await_status(shared: &Shared) -> DagStatusReport {
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
pub(super) async fn await_event(shared: &Shared, kind: &str, step: Option<&str>) -> DagEventIn {
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
pub(super) fn count_events(c: &Captured, kind: &str, step: &str) -> usize {
    c.events
        .iter()
        .filter(|e| e.kind == kind && e.step.as_deref() == Some(step))
        .count()
}

/// Per-test runtime inputs: uplink against the stub, a real LibsqlStore in
/// the temp dir, and the default Config for that workdir.
pub(super) struct Fixture {
    pub(super) container: container::ContainerFixture,
    pub(super) _bridge: model::ModelBridge,
    pub(super) uplink: Arc<Uplink>,
    pub(super) workdir: PathBuf,
    pub(super) workflow_root: PathBuf,
    pub(super) store: Arc<dyn opencoder_store::Store>,
    pub(super) config: opencoder_core::Config,
}

pub(super) async fn fixture(
    base: &str,
    tmp: &tempfile::TempDir,
    client: Arc<dyn opencoder_llm::ChatStream>,
) -> Fixture {
    let workdir = tmp.path().to_path_buf();
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(LibsqlStore::open(workdir.join("store.db")).await.unwrap());
    let mut config = opencoder_core::Config::default();
    let container = container::ContainerFixture::open(tmp.path());
    container.configure(&mut config);
    config.agent.agents_dir = Some(tmp.path().join("agents"));
    std::fs::create_dir_all(config.agent.agents_dir.as_ref().unwrap()).unwrap();
    let bridge = model::ModelBridge::start(client);
    bridge.configure(&mut config);
    config.local_memory = false;
    Fixture {
        container,
        _bridge: bridge,
        uplink: Arc::new(Uplink::new(base, "test-token").unwrap()),
        workdir: workdir.clone(),
        workflow_root: tmp.path().join("workflow"),
        store,
        config,
    }
}
