#![cfg(target_os = "linux")]
#[path = "harness/runc_fixture.rs"]
mod fixture;
#[path = "harness/registered.rs"]
mod registered;
mod support;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use support::*;

async fn save(fleet: &Fleet, spec: Value) {
    let reply = fleet
        .call("POST", "/api/dag/defs", json!({"spec":spec}))
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
}
async fn dispatch(fleet: &Fleet, id: &str, input: Value) -> opencoder_core::fleet::RpcReply {
    fleet
        .call(
            "POST",
            "/api/dag/defs/codex-runc/dispatch",
            json!({"id":id,"input":input,"node_id":fleet.nodes[0].registration().id}),
        )
        .await
}
fn one(prompt: &str, timeout: Option<u64>) -> Value {
    json!({"name":"codex-runc","steps":[{"name":"probe","timeout_secs":timeout,
        "kind":{"type":"agent","agent":"codex-runc","prompt":prompt}}]})
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

async fn prepared_container_timeout(root: &Path, config: &opencoder_core::Config) {
    let run = opencoder_dag::DagClaimedRun {
        run_id: "dag-process-timeout".into(),
        dag_id: "cleanup".into(),
        created_at: 0,
        spec: serde_json::from_value(json!({"name":"cleanup","steps":[]})).unwrap(),
    };
    let container = opencoder_dag_runtime::sandbox::run::RunContainer::start(root, config, &run)
        .await
        .unwrap();
    let process = opencoder_dag_runtime::sandbox::run::StepProcess {
        key: "timeout".into(),
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo started > timeout-started; exec /usr/bin/sleep 120".into(),
        ],
        env: vec![],
        cwd: "/workspace".into(),
        timeout_secs: Some(1),
    };
    let error = opencoder_dag_runtime::sandbox::run::execute(
        root,
        process,
        tokio_util::sync::CancellationToken::new(),
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("DAG step timeout"), "{error:#}");
    assert!(root.join("workspace/timeout-started").is_file());
    let process = opencoder_dag_runtime::sandbox::run::StepProcess {
        key: "healthy".into(),
        argv: vec!["/bin/sh".into(), "-c".into(), "printf alive".into()],
        env: vec![],
        cwd: "/workspace".into(),
        timeout_secs: Some(10),
    };
    assert_eq!(
        opencoder_dag_runtime::sandbox::run::execute(
            root,
            process,
            tokio_util::sync::CancellationToken::new(),
            None
        )
        .await
        .unwrap(),
        (0, "alive".into())
    );
    container.cleanup().await.unwrap();
    support::native::container::assert_runtime_released(root);
}
async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(180), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Codex container never started");
}

async fn settled(worker: &opencoder_worker::Worker, id: &str) -> Value {
    // Sandbox preparation binds the host mapping and mounts the per-run
    // overlay; on shared CI disks this can exceed the lightweight
    // host-harness helper's 20-second window.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let reply = worker
            .handle(opencoder_core::fleet::NodeOperation::Inspect {
                execution: execution_ref(worker, id).await,
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        if !matches!(
            reply.body["execution"]["status"].as_str(),
            Some("pending" | "running" | "cancelling")
        ) {
            return reply.body;
        }
        assert!(tokio::time::Instant::now() < deadline, "{id}: {reply:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// One current-thread test owns the process environment; all credentials are
// temporary fixtures. The actual sandbox (OCI runtime or native clone3) and
// kernel mounts are exercised on the backend `DAG_TEST_SANDBOX` selects.
#[tokio::test]
async fn server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation() {
    if opencoder_core::config::DagSandbox::from_test_env()
        == opencoder_core::config::DagSandbox::Runc
        && !opencoder_dag_runtime::sandbox::runc::runc_available()
    {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    // Build before installing the fixture HOME so rustup retains its host setup.
    let runner = fixture::runner();
    let temp = tempfile::tempdir().unwrap();
    let _env = fixture::Environment::new(temp.path());
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let node_root = fleet.root().join("n0/node/dag");
    // The runners directory is bound read-only at /run/opencoder/bin: the
    // DAG runners and the fake Codex CLI share it; nothing else is installed.
    let runners = node_root.join("runners");
    fixture::runners(&runners, &runner);
    let knowledge = temp.path().join("knowledge");
    std::fs::create_dir(&knowledge).unwrap();
    std::fs::write(knowledge.join("guide.txt"), "read-only guide").unwrap();
    std::fs::write(
        fleet.root().join("n0/work/.opencoder/config.json"),
        json!({"model":"unconfigured/model","providers":{"unconfigured":{"api_key":""}},
        "agent":{"codex":{"executable":"/run/opencoder/bin/codex"}},
        "dag":{"knowledge_root":knowledge,"runners_dir":runners}})
        .to_string(),
    )
    .unwrap();
    save(&fleet, json!({"name":"codex-runc","steps":[
        {"name":"first","kind":{"type":"agent","agent":"codex-runc","prompt":"CODEX_FIRST"}},
        {"name":"second","depends_on":["first"],"kind":{"type":"agent","agent":"codex-runc","prompt":"CODEX_SECOND"}}
    ]})).await;
    let accepted = dispatch(&fleet, "dag-codex-runc-default", json!({})).await;
    assert_eq!(accepted.status, 202, "{accepted:?}");
    let result = settled(&fleet.nodes[0], "dag-codex-runc-default").await;
    assert_eq!(result["execution"]["status"], "done", "{result}");
    for step in ["first", "second"] {
        let accepted_root =
            support::dag_run(&fleet.root().join("n0/node"), "dag-codex-runc-default");
        let dir = accepted_root.join(step);
        assert_eq!(read(&dir.join("output.json")), json!({"runc":true}));
        let session = read(&dir.join("session.json"));
        assert_eq!(session["harness"], "codex");
        assert_eq!(session["thread_id"], "runc-codex-thread");
        assert!(std::fs::read_to_string(
            accepted_root.join("upper").join(step).join("events.ndjson")
        )
        .unwrap()
        .contains("runc-tool-ok"));
        let config = read(&accepted_root.join("bundle/config.json"));
        assert!(!config["process"]["env"]
            .to_string()
            .contains("OPENAI_API_KEY="));
        support::native::container::assert_runtime_released(&accepted_root);
    }
    assert_eq!(
        std::fs::read_to_string(temp.path().join("home/.codex/auth.json")).unwrap(),
        "fixture-login"
    );
    assert!(temp.path().join("home/.codex/refresh-observed").is_file());
    assert!(!knowledge.join("write-probe").exists());

    registered::verify(&fleet).await;

    // Profile settings originate at Server, while their credential path points
    // to the selected node. The CLI executable and the startup program are
    // guest paths of the sandbox host mapping: both live in the runners
    // directory, which every sandbox binds read-only at /run/opencoder/bin
    // (the default /usr/bin/codex would be the host's own CLI).
    let profile_home = temp.path().join("profile-login");
    std::fs::create_dir(&profile_home).unwrap();
    std::fs::write(profile_home.join("auth.json"), "fixture-login").unwrap();
    let reply = fleet
        .call(
            "PUT",
            "/api/harnesses/codex/profiles/selected",
            json!({
                "model":"profile-model","reasoning_effort":"high",
                "executable":"/run/opencoder/bin/codex",
                "startup_script":["/bin/sh","/run/opencoder/bin/codex-start.sh"],
                "envs":{"CODEX_HOME":profile_home,"NOTE":"private-profile-fixture"}
            }),
        )
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    std::fs::write(fleet.root().join("n0/node/native-resources/source/agents/codex-runc/meta.json"),
        json!({"name":"codex-runc","harness":"codex","harness_profile":"selected","current":{"prompt":"probe"}}).to_string()).unwrap();
    save(&fleet, json!({"name":"codex-runc","steps":[{"name":"batch","kind":{
        "type":"dynamic","source":{"type":"input","pointer":"/items"},
        "template":{"type":"agent","agent":"codex-runc","prompt":"CODEX_FIRST","model":"step-model"}
    }}]})).await;
    assert_eq!(
        dispatch(
            &fleet,
            "dag-codex-runc-batch",
            json!({"items":["one","two"]})
        )
        .await
        .status,
        202
    );
    let result = settled(&fleet.nodes[0], "dag-codex-runc-batch").await;
    assert_eq!(result["execution"]["status"], "done", "{result}");
    for index in 0..2 {
        let dir = support::dag_run(&fleet.root().join("n0/node"), "dag-codex-runc-batch")
            .join(format!("upper/batch/instances/{index}"));
        let args = std::fs::read_to_string(dir.join("argv.txt")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("startup-state")).unwrap(),
            "before\nafter\n"
        );
        let context = read(&dir.join("startup-context.json"));
        assert_eq!(context["agent"], "codex-runc");
        assert_eq!(context["schema_version"], 1);
        assert!(context["session_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        for expected in ["--model\nstep-model", "model_reasoning_effort=\"high\""] {
            assert!(args.contains(expected), "{args}");
        }
        let reply = fleet
            .call(
                "GET",
                &format!("/api/dag/runs/dag-codex-runc-batch/steps/batch/instances/{index}"),
                Value::Null,
            )
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        assert!(!reply.body.to_string().contains("private-profile-fixture"));
    }
    assert!(profile_home.join("refresh-observed").is_file());

    for (id, prompt, timeout) in [
        ("dag-codex-runc-auth-fail", "CODEX_AUTH_FAIL", None),
        ("dag-codex-runc-malformed", "CODEX_MALFORMED", None),
        ("dag-codex-runc-timeout", "CODEX_WAIT", Some(2)),
    ] {
        save(&fleet, one(prompt, timeout)).await;
        let accepted = dispatch(&fleet, id, json!({})).await;
        assert_eq!(accepted.status, 202, "{accepted:?}");
        let result = settled(&fleet.nodes[0], id).await;
        assert_eq!(result["execution"]["status"], "error", "{result}");
        let accepted_root = support::dag_run(&fleet.root().join("n0/node"), id);
        let meta = read(&accepted_root.join("probe/meta.json"));
        let expected = match prompt {
            "CODEX_AUTH_FAIL" => "fixture authentication rejected",
            "CODEX_MALFORMED" => "invalid Codex JSONL",
            "CODEX_WAIT" => "step timeout after 2s",
            _ => unreachable!(),
        };
        assert!(meta["error"].as_str().unwrap().contains(expected), "{meta}");
        // The DAG deadline includes sandbox staging; the OCI backend creates
        // no state if cancellation arrives before container launch. A native
        // run always records its init, which must be gone.
        let state = accepted_root.join("runc-state");
        if timeout.is_none()
            || state.exists()
            || opencoder_core::config::DagSandbox::from_test_env()
                != opencoder_core::config::DagSandbox::Runc
        {
            support::native::container::assert_runtime_released(&accepted_root);
        }
    }
    save(&fleet, one("CODEX_WAIT", None)).await;
    assert_eq!(
        dispatch(&fleet, "dag-codex-runc-cancel", json!({}))
            .await
            .status,
        202
    );
    let cancel_root = loop {
        let journal = fleet
            .root()
            .join("n0/node/dag/dag-codex-runc-cancel/execution.json");
        if journal.is_file() {
            break support::dag_run(&fleet.root().join("n0/node"), "dag-codex-runc-cancel");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    wait_file(&cancel_root.join("workspace/probe/started")).await;
    let cancelled = fleet
        .call(
            "POST",
            "/api/dag/runs/dag-codex-runc-cancel/cancel",
            Value::Null,
        )
        .await;
    assert_eq!(cancelled.status, 200, "{cancelled:?}");
    let result = settled(&fleet.nodes[0], "dag-codex-runc-cancel").await;
    assert_eq!(result["execution"]["status"], "cancelled", "{result}");
    support::native::container::assert_runtime_released(&cancel_root);

    let config = opencoder_core::Config::load(&fleet.root().join("n0/work")).unwrap();
    prepared_container_timeout(&node_root.join("cleanup-probe"), &config).await;
    // Without the CLI in the runners directory the host mapping cannot reach
    // /run/opencoder/bin/codex; admission fails before any container starts.
    std::fs::rename(runners.join("codex"), runners.join("codex-disabled")).unwrap();
    let rejected = dispatch(&fleet, "dag-codex-runc-missing-cli", json!({})).await;
    assert_eq!(rejected.status, 400, "{rejected:?}");
    assert!(rejected
        .body
        .to_string()
        .contains("Codex executable missing on the host at /run/opencoder/bin/codex"));
    assert_eq!(
        client.call_count(),
        0,
        "Codex must not call the native provider"
    );
    fleet.shutdown().await;
}
