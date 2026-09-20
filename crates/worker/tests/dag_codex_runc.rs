#![cfg(target_os = "linux")]
#[path = "harness/runc_fixture.rs"]
mod fixture;
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

async fn prepared_container_timeout(node_root: &Path) {
    // Reuse a completed bundle so image staging cannot consume this budget.
    // The DAG cancellation case below separately exercises the running Codex
    // session; this probe checks runc's own deadline and process-tree cleanup.
    let bundle = node_root.join("bundles/dag-codex-runc-default/first");
    let config_path = bundle.join("config.json");
    let mut config = read(&config_path);
    config["process"]["args"] = json!([
        "/bin/sh",
        "-c",
        r#"printf started > "$OPENCODER_STEP_DIR/timeout-started"; exec /usr/bin/sleep 120"#,
    ]);
    std::fs::write(config_path, config.to_string()).unwrap();
    let marker = node_root.join("dag-codex-runc-default/first/timeout-started");
    let cancel = tokio_util::sync::CancellationToken::new();
    let execution = opencoder_dag_runtime::sandbox::runc::run_step_cancellable(
        &bundle,
        "codex-prepared-timeout",
        Some(5),
        cancel.clone(),
    );
    tokio::pin!(execution);
    let initial = futures::poll!(&mut execution);
    assert!(initial.is_pending(), "{initial:?}");
    // runc now runs independently of this current-thread reactor. Hold off
    // polling its deadline until the shell starts, so slow OCI startup cannot
    // turn this running-container cleanup check into a pre-start timeout.
    let startup = std::time::Instant::now();
    while !marker.is_file() && startup.elapsed() < Duration::from_secs(180) {
        std::thread::sleep(Duration::from_millis(30));
    }
    if !marker.is_file() {
        cancel.cancel();
    }
    let error = execution.await.unwrap_err();
    assert!(marker.is_file(), "container never started: {error:#}");
    assert!(error.to_string().contains("runc step timeout"), "{error:#}");
    assert!(std::fs::read_dir(bundle.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
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
    // Container preparation copies a real rootfs. On shared CI disks this
    // can exceed the lightweight host-harness helper's 20-second window.
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
// temporary fixtures. The actual OCI runner and kernel mounts are exercised.
#[tokio::test]
async fn server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation() {
    if !opencoder_dag_runtime::sandbox::runc::runc_available() {
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
    let rootfs = node_root.join("rootfs");
    fixture::rootfs(&rootfs, &runner);
    let knowledge = temp.path().join("knowledge");
    std::fs::create_dir(&knowledge).unwrap();
    std::fs::write(knowledge.join("guide.txt"), "read-only guide").unwrap();
    std::fs::write(
        fleet.root().join("n0/work/.opencoder/config.json"),
        json!({"model":"unconfigured/model","providers":{"unconfigured":{"api_key":""}},
        "dag":{"agent_sandbox":"runc","knowledge_root":knowledge}})
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
        let dir = node_root.join("dag-codex-runc-default").join(step);
        assert_eq!(read(&dir.join("output.json")), json!({"runc":true}));
        let session = read(&dir.join("session.json"));
        assert_eq!(session["harness"], "codex");
        assert_eq!(session["thread_id"], "runc-codex-thread");
        assert!(std::fs::read_to_string(dir.join("events.ndjson"))
            .unwrap()
            .contains("runc-tool-ok"));
        let bundle = node_root.join("bundles/dag-codex-runc-default").join(step);
        let config = read(&bundle.join("config.json"));
        assert!(!config["process"]["env"]
            .to_string()
            .contains("OPENAI_API_KEY="));
        assert!(std::fs::read_dir(bundle.join("runc-state"))
            .unwrap()
            .next()
            .is_none());
    }
    assert_eq!(
        std::fs::read_to_string(temp.path().join("home/.codex/auth.json")).unwrap(),
        "fixture-login"
    );
    assert!(temp.path().join("home/.codex/refresh-observed").is_file());
    assert!(!knowledge.join("write-probe").exists());

    // Profile settings originate at Server, while their credential path points
    // to the selected node. The CLI executable is resolved INSIDE the rootfs.
    let profile_home = temp.path().join("profile-login");
    std::fs::create_dir(&profile_home).unwrap();
    std::fs::write(profile_home.join("auth.json"), "fixture-login").unwrap();
    let reply = fleet
        .call(
            "PUT",
            "/api/harnesses/codex/profiles/selected",
            json!({
                "model":"profile-model","auth_slot":2,"reasoning_effort":"high",
                "envs":{"CODEX_HOME":profile_home,"NOTE":"private-profile-fixture"}
            }),
        )
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    std::fs::write(temp.path().join("agents/codex-runc/meta.json"),
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
        let dir = node_root.join(format!("dag-codex-runc-batch/batch/instances/{index}"));
        let args = std::fs::read_to_string(dir.join("argv.txt")).unwrap();
        for expected in [
            "--auth-slot\n2",
            "--model\nstep-model",
            "model_reasoning_effort=\"high\"",
        ] {
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
        let meta = read(&node_root.join(format!("{id}/probe/meta.json")));
        let expected = match prompt {
            "CODEX_AUTH_FAIL" => "fixture authentication rejected",
            "CODEX_MALFORMED" => "invalid Codex JSONL",
            "CODEX_WAIT" => "step timeout after 2s",
            _ => unreachable!(),
        };
        assert!(meta["error"].as_str().unwrap().contains(expected), "{meta}");
        let state = node_root.join(format!("bundles/{id}/probe/runc-state"));
        // The DAG deadline includes image staging; no runc state is created
        // if cancellation arrives before container launch.
        if timeout.is_none() || state.exists() {
            assert!(std::fs::read_dir(state).unwrap().next().is_none());
        }
    }
    save(&fleet, one("CODEX_WAIT", None)).await;
    assert_eq!(
        dispatch(&fleet, "dag-codex-runc-cancel", json!({}))
            .await
            .status,
        202
    );
    wait_file(&node_root.join("dag-codex-runc-cancel/probe/started")).await;
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
    assert!(
        std::fs::read_dir(node_root.join("bundles/dag-codex-runc-cancel/probe/runc-state"))
            .unwrap()
            .next()
            .is_none()
    );

    std::fs::rename(
        rootfs.join("usr/bin/codex"),
        rootfs.join("usr/bin/codex-disabled"),
    )
    .unwrap();
    let rejected = dispatch(&fleet, "dag-codex-runc-missing-cli", json!({})).await;
    assert_eq!(rejected.status, 400, "{rejected:?}");
    assert!(rejected
        .body
        .to_string()
        .contains("executable missing in rootfs"));
    assert_eq!(
        client.call_count(),
        0,
        "Codex must not call the native provider"
    );
    prepared_container_timeout(&node_root).await;
    fleet.shutdown().await;
}
