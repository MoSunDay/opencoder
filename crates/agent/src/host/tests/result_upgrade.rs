//! Explicit native cross-version gate; the old binary must be a released artifact.
use super::*;
use base64::Engine;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
};
#[path = "result_upgrade/control.rs"]
mod control;
type CollectedResult<'a> = (&'a str, ExecutionKind, &'a str, (String, Vec<u8>));

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn settled_native(host: &Host, id: &str, kind: ExecutionKind) {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let reply = host
                .handle(NodeOperation::Inspect {
                    execution: ExecutionRef {
                        id: id.into(),
                        kind,
                    },
                })
                .await;
            let status = reply.body["execution"]["status"].as_str();
            assert!(
                !matches!(status, Some("error" | "cancelled" | "interrupted")),
                "{reply:?}"
            );
            if matches!(status, Some("done" | "idle")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
}

async fn full(host: &Host, id: &str, kind: ExecutionKind, field: &str) -> (String, Vec<u8>) {
    let mut offset = 0;
    let mut result = Vec::new();
    let mut version = None;
    loop {
        let reply = host
            .handle(NodeOperation::DetailField {
                request: DetailFieldRequest {
                    execution: ExecutionRef {
                        id: id.into(),
                        kind,
                    },
                    field: field.into(),
                    offset,
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let current = reply.body["version"].as_str().unwrap().to_owned();
        assert_eq!(version.get_or_insert(current.clone()), &current);
        assert_eq!(reply.body["offset"], offset);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(reply.body["bytes_b64"].as_str().unwrap())
            .unwrap();
        assert!(bytes.len() <= EVENT_CHUNK_BYTES);
        offset += bytes.len() as u64;
        assert_eq!(reply.body["next_offset"], offset);
        result.extend(bytes);
        if reply.body["eof"] == true {
            assert_eq!(reply.body["total_bytes"], offset);
            break;
        }
    }
    if kind == ExecutionKind::Team {
        assert!(
            result.len() > 128 * 1024,
            "Team must exercise multi-chunk reads"
        );
    } else {
        // This release saves only the trailing 8 KiB of Agent output.
        let saved: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(saved["output_text"].as_str().unwrap().len(), 8192);
    }
    (version.unwrap(), result)
}

fn team(host: &Host, id: &str) -> NodeOperation {
    let NodeOperation::Create { mut assignment } = create(host, id) else {
        unreachable!()
    };
    assignment.index.kind = ExecutionKind::Team;
    assignment.request.kind = ExecutionKind::Team;
    assignment.request.target = None;
    assignment.request.input = json!({"prompt":"review historical evidence"});
    assignment.definition =
        Some(json!({"name":"review","captain":"act","members":[{"agent":"act"},{"agent":"plan"}]}));
    NodeOperation::Create { assignment }
}

#[tokio::test]
#[ignore = "requires OPENCODER_OLD_AGENT pointing to a released binary without versioned fields"]
async fn released_runtime_results_survive_host_upgrade_sleep_wake_and_rollback() {
    let binary = std::env::var("OPENCODER_OLD_AGENT").expect("released old binary required");
    let info: serde_json::Value = serde_json::from_slice(
        &Command::new(&binary)
            .arg("--build-info")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(
        info["git_dirty"], false,
        "use an immutable release artifact"
    );
    let dir = tempfile::tempdir().unwrap().keep();
    println!("native cross-version evidence: {}", dir.display());
    let _scope = opencoder_core::config::scoped_config_home(dir.as_path().join("home"));
    let host = Host::open(
        &dir.as_path().join("host"),
        "mixed-version".into(),
        "fixture-token".into(),
        4,
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let hold = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let answer = json!({"question":"inspect","participants":["plan"],"summary":"aligned","aligned":true,"complete":true,
        "final_summary":format!("historical conclusion {} historical conclusion", "evidence ".repeat(20_000))}).to_string();
    let (held, seen, gate) = (hold.clone(), entered.clone(), release.clone());
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move |axum::Json(_request): axum::Json<serde_json::Value>| {
        let (held, seen, gate, answer) = (held.clone(), seen.clone(), gate.clone(), answer.clone());
        async move {
            if held.load(Ordering::SeqCst) { seen.fetch_add(1, Ordering::SeqCst); gate.notified().await; }
            let delta = json!({"choices":[{"index":0,"delta":{"role":"assistant","content":answer},"finish_reason":null}]});
            let end = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
            ([("content-type", "text/event-stream")], format!("data: {delta}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
        }
    })).layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024));
    let model = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let work = dir.as_path().join("work");
    let data = dir.as_path().join("old");
    std::fs::create_dir_all(work.join(".opencoder")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(work.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    std::fs::write(
        work.join("opencoder.json"),
        json!({"model":"fixture/model","context_limit":2_000_000,"cache_salt":false,
        "providers":{"fixture":{"base_url":format!("{endpoint}/v1"),"api_key":"fixture"}}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(data.join("node-id"), &host.registration.id).unwrap();
    std::fs::write(
        data.join("host-binding.json"),
        json!({"database":dir.as_path().join("host/host.db"),"runtime_id":"old"}).to_string(),
    )
    .unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let old_url = format!("http://127.0.0.1:{port}");
    host.store.register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord { id:"old".into(), release_id:info["git_commit"].as_str().unwrap().into(), mode:"staged".into(),
        config:json!({"endpoint":old_url,"data_dir":data,"unit":"opencoder-runtime-result-test.service"}) }).await.unwrap();
    host.store.activate_runtime("old").await.unwrap();
    let spawn = || {
        let log = std::fs::File::create(dir.as_path().join("old-runtime.log")).unwrap();
        Process(
            Command::new(&binary)
                .args(["--no-dag", "--workdir"])
                .arg(&work)
                .arg("--data-dir")
                .arg(&data)
                .args([
                    "--token",
                    "fixture-token",
                    "runtime",
                    "--port",
                    &port.to_string(),
                ])
                .env("HOME", dir.as_path())
                .env("XDG_CONFIG_HOME", dir.as_path().join("config"))
                .env("XDG_DATA_HOME", dir.as_path().join("data"))
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        )
    };
    let old = spawn();
    let ready = || async {
        host.client
            .get(format!("{old_url}/inventory"))
            .bearer_auth(&host.token)
            .send()
            .await
            .is_ok()
    };
    tokio::time::timeout(Duration::from_secs(60), async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    for (id, kind) in [
        ("agent-before", ExecutionKind::Agent),
        ("team-before", ExecutionKind::Team),
    ] {
        let operation = if kind == ExecutionKind::Team {
            team(&host, id)
        } else {
            create(&host, id)
        };
        let reply = host.handle(operation).await;
        assert_eq!(reply.status, 200, "{reply:?}");
        settled_native(&host, id, kind).await;
    }
    let unversioned: RpcReply = host
        .client
        .post(format!("{old_url}/rpc"))
        .bearer_auth(&host.token)
        .json(&NodeOperation::DetailField {
            request: DetailFieldRequest {
                execution: ExecutionRef {
                    id: "agent-before".into(),
                    kind: ExecutionKind::Agent,
                },
                field: "result".into(),
                offset: 0,
            },
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unversioned.status, 200);
    assert!(unversioned.body.get("version").is_none());
    hold.store(true, Ordering::SeqCst);
    assert_eq!(host.handle(create(&host, "agent-during")).await.status, 200);
    assert_eq!(host.handle(team(&host, "team-during")).await.status, 200);
    wait(|| async { entered.load(Ordering::SeqCst) >= 2 }).await;
    let (candidate, runtime_server) =
        runtime(&host, dir.as_path(), "new", Arc::new(MockChatClient::new())).await;
    host.store.activate_runtime("new").await.unwrap();
    let old_pid = old.0.id();
    hold.store(false, Ordering::SeqCst);
    release.notify_waiters();
    let mut expected = Vec::new();
    for (id, kind, field) in [
        ("agent-before", ExecutionKind::Agent, "result"),
        ("team-before", ExecutionKind::Team, "team.topic"),
        ("agent-during", ExecutionKind::Agent, "result"),
        ("team-during", ExecutionKind::Team, "team.topic"),
    ] {
        settled_native(&host, id, kind).await;
        assert_eq!(
            host.store.owner(id).await.unwrap().unwrap().runtime_id,
            "old"
        );
        expected.push((id, kind, field, full(&host, id, kind, field).await));
    }
    assert!(Path::new(&format!("/proc/{old_pid}")).exists());
    let control = control::Server::start(host.clone(), &dir).await;
    let links = control.collect(&expected).await;
    let inventory: serde_json::Value = host
        .client
        .get(format!("{old_url}/inventory"))
        .bearer_auth(&host.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inventory["can_hibernate"], true);
    host.store
        .put_definition("runtime_sleep", "old", &inventory)
        .await
        .unwrap();
    drop(old);
    // No wake request or mutation of the old Runtime is needed to read history.
    let restarted = Host::open(
        &dir.as_path().join("host"),
        "mixed-version".into(),
        "fixture-token".into(),
        4,
    )
    .await
    .unwrap();
    for (id, kind, field, value) in &expected {
        assert_eq!(full(&restarted, id, *kind, field).await, *value);
    }
    let _awakened = spawn();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    restarted
        .store
        .put_definition("runtime_sleep", "old", &serde_json::Value::Null)
        .await
        .unwrap();
    control.refresh(&links).await;
    restarted.store.activate_runtime("old").await.unwrap();
    assert_eq!(
        restarted
            .handle(create(&restarted, "agent-rollback"))
            .await
            .status,
        200
    );
    settled_native(&restarted, "agent-rollback", ExecutionKind::Agent).await;
    full(&restarted, "agent-rollback", ExecutionKind::Agent, "result").await;
    for (id, kind, field, value) in expected {
        assert_eq!(full(&restarted, id, kind, field).await, value);
    }
    println!("old release {}: completed + in-flight Agent/Team, unchanged owner/PID, Host restart, sleep, wake, rollback passed",info["git_commit"]);
    candidate.shutdown().await.unwrap();
    drop(control);
    runtime_server.abort();
    model.abort();
}
