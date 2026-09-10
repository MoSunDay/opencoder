//! Real binary + loopback Responses server, including a second process resume.
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn answer(text: &str) -> Value {
    json!({"type":"message","id":"msg_1","role":"assistant","status":"completed",
        "phase":"final_answer","content":[{"type":"output_text","text":text,"annotations":[]}]})
}

#[tokio::test]
async fn cli_edits_file_and_next_process_replays_responses_state() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        let reasoning = json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque-cli-fixture"});
        let tool = json!({"type":"function_call","id":"fc_1","call_id":"edit1","name":"edit","status":"completed",
            "arguments":json!({"path":"a.txt","old_string":"broken","new_string":"fixed"}).to_string()});
        let turns = vec![
            vec![reasoning, tool],
            vec![answer("CLI task done")],
            vec![answer("Fix file")],
            vec![answer("CLI resumed")],
            vec![answer("Continue fix")],
        ];
        for output in turns {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&buf[..n]);
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let header = String::from_utf8_lossy(&raw[..header_end]);
            assert!(header.starts_with("POST /responses "));
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            while raw.len() < header_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&buf[..n]);
            }
            captured
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&raw[header_end..header_end + length]).unwrap());
            let terminal = json!({"type":"response.completed","response":{"object":"response","id":"resp_1","status":"completed","output":output}});
            let body = format!("event: response.completed\ndata: {terminal}\n\n");
            let reply=format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    std::fs::create_dir_all(dir.path().join(".opencoder")).unwrap();
    std::fs::write(dir.path().join("a.txt"), "broken").unwrap();
    std::fs::write(
        dir.path().join(".opencoder/config.json"),
        json!({"model":"fixture/gpt-6",
        "providers":{"fixture":{"protocol":"responses","base_url":url,"api_key":"fixture-key"}}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.path().join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    for resume in [false, true] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_opencoder"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", dir.path())
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .current_dir(dir.path())
            .arg("--workdir")
            .arg(dir.path())
            .kill_on_drop(true);
        if resume {
            command.arg("--continue");
        }
        command.args(["run", if resume { "Continue" } else { "Fix a.txt" }]);
        let output = tokio::time::timeout(Duration::from_secs(30), command.output())
            .await
            .expect("CLI timed out")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(if resume {
            "CLI resumed"
        } else {
            "CLI task done"
        }));
    }
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "fixed"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[0]["model"], "gpt-6");
    let input = requests[3]["input"].as_array().unwrap();
    assert!(input
        .iter()
        .any(|v| v["encrypted_content"] == "opaque-cli-fixture"));
    assert!(input
        .iter()
        .any(|v| v["type"] == "function_call_output" && v["call_id"] == "edit1"));
    assert!(input.iter().any(|v| v["phase"] == "final_answer"));
}
