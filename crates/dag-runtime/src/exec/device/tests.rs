use super::*;

#[test]
fn endpoint_rejects_credentials_and_query_secrets() {
    assert!(endpoint("http://127.0.0.1:18109").is_ok());
    for value in [
        "http://user:secret@host",
        "http://host?token=secret",
        "file:///private",
    ] {
        assert!(endpoint(value).is_err());
    }
}

#[test]
fn prompt_mentions_transport_path_without_credential_contents() {
    let access = Access {
        root: PathBuf::from("/private/session"),
        identity: json!({"session_id":"host-session"}),
        input: json!({"device_count":1}),
        config: DagDeviceConfig {
            host_endpoint: "http://127.0.0.1:18109".into(),
            step_endpoint: "http://127.0.0.1:18109".into(),
            token_file: "/private/root-token".into(),
        },
    };
    let text = access.prompt("task".into(), false);
    assert!(text.contains("/private/session/transport.json"));
    assert!(text.contains("host-session"));
    assert!(!text.contains("capability"));
    let guest = access.prompt("task".into(), true);
    assert!(guest.contains("/run/opencoder-device/transport.json"));
    assert!(!guest.contains("/private/session"));
}

fn context(root: &Path, input: &Value, instance: Option<usize>) -> StepCtx {
    let spec = opencoder_dag::devices::definition(input).unwrap();
    StepCtx {
        run_id: "dag-fixture".into(),
        instance,
        instance_input: None,
        step: spec.steps[usize::from(instance.is_some())].clone(),
        spec,
        states: Default::default(),
        outputs: Default::default(),
        workflow_root: root.into(),
        log: None,
        knowledge_root: None,
        ops: Default::default(),
    }
}

#[test]
fn immutable_dispatch_rejects_changed_cases_and_caller_identity_is_not_trusted() {
    let temp = tempfile::tempdir().unwrap();
    let input = json!({"device_count":1,"case_ids":["a"],"session_id":"forged","dag_id":"other"});
    let mut ctx = context(temp.path(), &input, None);
    std::fs::create_dir_all(temp.path().join("dag-fixture")).unwrap();
    let path = temp.path().join("dag-fixture/input.json");
    std::fs::write(&path, input.to_string()).unwrap();
    assert_eq!(
        controlled_input(&ctx).unwrap().unwrap(),
        json!({"device_count":1,"case_ids":["a"]})
    );
    let who = identity(&ctx, "actual-host-session");
    assert_eq!(who["session_id"], "actual-host-session");
    assert_eq!(who["dag_id"], "dag-fixture");
    std::fs::write(
        &path,
        json!({"device_count":1,"case_ids":["b"]}).to_string(),
    )
    .unwrap();
    assert!(controlled_input(&ctx).is_err());
    std::fs::write(&path, input.to_string()).unwrap();
    ctx.spec.steps[0].timeout_secs = Some(999);
    assert!(controlled_input(&ctx).is_err());
    // Ordinary workflows do not read input files or require device configuration.
    ctx.spec.name = "ordinary".into();
    std::fs::remove_file(path).unwrap();
    assert!(controlled_input(&ctx).unwrap().is_none());
}

#[test]
fn assignment_is_bound_to_instance_machine_and_frozen_batch() {
    let input = json!({"device_count":2,"case_ids":["a","b","c"]});
    let mut ctx = context(Path::new("/unused"), &input, Some(0));
    let valid = json!({"instance_id":"0","machine":"win-19","generation":1,
        "reservation_id":"reservation-123","case_ids":["a","c"]});
    ctx.instance_input = Some(Value::String(valid.to_string()));
    assert_eq!(assignment(&ctx, &input).unwrap(), valid);
    for (key, value) in [
        ("instance_id", json!("1")),
        ("machine", json!("win-01")),
        ("case_ids", json!(["b"])),
        ("generation", json!(null)),
        ("reservation_id", json!("../../else")),
    ] {
        let mut forged = valid.clone();
        forged[key] = value;
        ctx.instance_input = Some(Value::String(forged.to_string()));
        assert!(assignment(&ctx, &input).is_err(), "accepted forged {key}");
    }
}

#[test]
fn device_transport_mount_is_read_only_and_separate_from_public_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("bundle");
    let private = temp.path().join("private");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(bundle.join("config.json"), r#"{"mounts":[]}"#).unwrap();
    super::super::private_files::bind_at(&bundle, &private, GUEST).unwrap();
    let config: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["mounts"][0]["destination"], GUEST);
    assert!(config["mounts"][0]["options"]
        .as_array()
        .unwrap()
        .contains(&json!("ro")));
    assert!(super::super::private_files::bind_at(&bundle, &private, GUEST).is_err());
}

#[tokio::test]
async fn host_finish_sends_exact_identity_without_exposing_api_response() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let token = temp.path().join("token");
    std::fs::write(&token, "fixture-host-secret").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let reader = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut data = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if body.len() >= length {
                    break;
                }
            }
        }
        stream.write_all(b"HTTP/1.1 409 Conflict\r\nContent-Length: 19\r\nConnection: close\r\n\r\nfixture-api-secret!").unwrap();
        String::from_utf8(data).unwrap()
    });
    let access = Access {
        root: temp.path().into(),
        identity: json!({"dag_id":"d","step_id":"execute","instance_id":"0","session_id":"real-session"}),
        input: json!({}),
        config: DagDeviceConfig {
            host_endpoint: format!("http://{address}"),
            step_endpoint: format!("http://{address}"),
            token_file: token,
        },
    };
    let mut result = super::super::StepResult {
        outcome: opencoder_dag::StepOutcome::Cancelled,
        error: None,
        output_text: String::new(),
        output_json: None,
        session_id: Some("real-session".into()),
    };
    access.finish(&mut result).await;
    let request = reader.join().unwrap();
    assert!(request.starts_with("POST /v1/step-sessions/real-session/finish "));
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({"dag_id":"d","step_id":"execute","instance_id":"0","session_id":"real-session","outcome":"cancelled"})
    );
    assert_eq!(result.outcome, opencoder_dag::StepOutcome::Error);
    let error = result.error.unwrap();
    assert!(error.contains("409"));
    assert!(!error.contains("fixture-host-secret") && !error.contains("fixture-api-secret"));
}
