//! Inputs accepted while a real sandbox round is running must stay in that session.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[test]
fn sandbox_accepts_active_guidance_and_queue_without_duplicate_retries() {
    if !runc_available() {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    let released = Arc::new(AtomicBool::new(false));
    let gate = released.clone();
    let stub = LlmStub::spawn(vec![
        Script::dynamic(move |_| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !gate.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            "initial reply".into()
        }),
        Script::Text("queued reply".into()),
        Script::Text("later reply".into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    write_card(
        tmp.path(),
        "boxer",
        "boxer-pool",
        "agent",
        "# agent\nAnswer the user.",
        None,
    );
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "sandbox-inputs");
    fleet.wait_ready(&["agent", "operator", "dag"]);
    prepare_rootfs(&fleet);
    let id = "agent-active-inputs";
    let (status,body)=fleet.http("POST","/api/sessions",&json!({"id":id,"kind":"agent","agent":"boxer","node_id":fleet.node_id(),"prompt":"first task"}));
    assert_eq!(status, 200, "{body}");
    stub.wait_for_requests(1);
    for (delivery, input_id, prompt) in [
        ("steer", "guidance", "extra context"),
        ("queue", "queued", "second task"),
    ] {
        let body = json!({"prompt":prompt,"input_id":input_id,"delivery":delivery});
        let (status, first) = fleet.http("POST", &format!("/api/sessions/{id}/prompt"), &body);
        assert_eq!(status, 200, "{first}");
        let (status, retry) = fleet.http("POST", &format!("/api/sessions/{id}/prompt"), &body);
        assert_eq!(status, 200, "{retry}");
        assert_eq!(retry["seq"], first["seq"]);
        assert_eq!(retry["inserted"], false);
    }
    released.store(true, Ordering::SeqCst);
    let done = fleet.wait_idle(id);
    assert_eq!(done["execution"]["node_id"], fleet.node_id());
    let (_, session) = get(&fleet, &format!("/api/sessions/{id}"));
    let text = session["messages"].to_string();
    let user_texts: Vec<_> = session["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .flat_map(|message| message["blocks"].as_array().unwrap())
        .filter_map(|block| block["text"].as_str())
        .collect();
    assert_eq!(user_texts.len(), 3, "{text}");
    for prompt in ["first task", "extra context", "second task"] {
        assert_eq!(
            user_texts.iter().filter(|text| **text == prompt).count(),
            1,
            "{text}"
        );
    }
    assert!(text.contains("queued reply"), "{text}");
    let requests = stub.wait_for_requests(2);
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("second task") && requests[1].contains("extra context"));
    let (_, retry) = fleet.http(
        "POST",
        &format!("/api/sessions/{id}/prompt"),
        &json!({"prompt":"second task","input_id":"queued","delivery":"queue"}),
    );
    assert_eq!(retry["inserted"], false);
    assert_eq!(stub.request_count(), 2);
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{id}/prompt"),
        &json!({"prompt":"continue later","input_id":"later"}),
    );
    assert_eq!(status, 200, "{body}");
    fleet.wait_idle(id);
    let requests = stub.wait_for_requests(3);
    assert!(requests[2].contains("extra context") && requests[2].contains("continue later"));
}
