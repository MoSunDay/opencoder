use super::*;
use opencoder_core::ContentBlock;

#[tokio::test]
async fn chunked_transcript_and_large_tool_payload_are_reassembled_exactly() {
    let fixture = Fixture::new().await;
    let text = "汉字".repeat(5000);
    let mut message = Message::user("u", &text);
    message.blocks.push(ContentBlock::Image {
        url: "data:image/png;base64,AA==".into(),
        detail: None,
    });
    message.display = Some("original $skill @file".into());
    message.synthetic = true;
    message.usage.total_tokens = 321;
    fixture.state.lock().unwrap().messages.push(message);
    let payload = json!({"id":"call","name":"exec_command","output":text,"is_error":false});
    fixture.state.lock().unwrap().payload = serde_json::to_vec(&payload).unwrap();
    let client = client::ServerClient::with_token(&fixture.url, None, None).unwrap();
    let messages = transcript::messages(&client, "agent-test").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text(), text);
    assert_eq!(messages[0].blocks.len(), 2);
    assert_eq!(
        messages[0].display.as_deref(),
        Some("original $skill @file")
    );
    assert!(messages[0].synthetic);
    assert_eq!(messages[0].usage.total_tokens, 321);
    assert_eq!(
        transcript::event_data(&client, "agent-test", Some(7), json!({"omitted":true}))
            .await
            .unwrap(),
        payload
    );
    assert!(
        transcript::event_data(&client, "agent-test", None, json!({"omitted":true}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn remote_catalog_filters_other_kinds_and_reports_permission_failure() {
    let fixture = Fixture::new().await;
    let cards = catalog(&fixture.config()).await.unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].name, "registered-codex");
    assert!(cards[1].description.starts_with("operator"));
    let pick = select(&fixture.config(), "registered-codex").await.unwrap();
    assert!(
        matches!(pick,crate::task::TaskPick::Remote(binding) if binding.capability.target=="codex")
    );
    assert!(select(&fixture.config(), "missing").await.is_err());
    fixture.state.lock().unwrap().auth_denied = true;
    let error = catalog(&fixture.config()).await.unwrap_err().to_string();
    assert!(error.contains("denied"));
    assert!(matches!(
        select(&fixture.config(), "self").await.unwrap(),
        crate::task::TaskPick::New
    ));
}

#[test]
fn fragmented_sse_handles_utf8_crlf_comments_multiline_data_and_reconnect() {
    let mut decoder = stream::Decoder::default();
    let bytes="event: text_delta\r\nid: 2\r\ndata: {\r\ndata: \"text\":\"你好\"}\r\n\r\n: keepalive\n\nevent: reconnect\ndata: release switch\n\n".as_bytes();
    let mut frames = Vec::new();
    for byte in bytes {
        frames.extend(decoder.push(std::slice::from_ref(byte)).unwrap());
    }
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].seq, Some(2));
    assert_eq!(frames[0].data["text"], "你好");
    assert_eq!(frames[1].kind, "reconnect");
    assert!(decoder
        .push(b"event: text_delta\ndata: invalid\n\n")
        .is_err());
}

#[tokio::test]
async fn credentials_only_go_to_http_and_questions_use_existing_hub_without_skip_on_detach() {
    let fixture = Fixture::new().await;
    let client =
        client::ServerClient::with_token(&fixture.url, None, Some("fixture-token".into())).unwrap();
    let response: serde_json::Value = client.get("/api/tui/agent-capabilities").await.unwrap();
    assert_eq!(response["authorization"], "Bearer fixture-token");
    let hub = opencoder_session::QuestionHub::new();
    hub.attach();
    let detach = tokio_util::sync::CancellationToken::new();
    let (ui, mut events) = tokio::sync::mpsc::channel(16);
    let bridge = questions::QuestionBridge {
        client,
        execution: "agent-question".into(),
        hub: hub.clone(),
        ui,
        detach: detach.clone(),
    };
    fixture.state.lock().unwrap().questions =
        vec![json!({"id":"q1","question":"Continue?","options":["yes","no"]})];
    bridge.restore().await;
    assert!(
        matches!(events.recv().await.unwrap(),UiEvent::Session(SessionEvent::ToolStart{id,..}) if id=="q1")
    );
    assert!(hub.resolve("q1", "yes".into()));
    wait_for(&fixture, |requests| {
        requests.iter().any(|(_, b)| {
            b["input"]["tail"] == "questions/q1/answer" && b["input"]["body"]["answer"] == "yes"
        })
    })
    .await;
    bridge.start("q2".into(), &json!({"question":"Still waiting?"}));
    detach.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !hub.waiting_questions().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!fixture
        .state
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(|(_, b)| b["input"]["tail"] == "questions/q2/answer"));
}

#[tokio::test]
async fn reconnect_resumes_cursor_and_never_redelivers_frames() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().events = vec![frame(1, SessionEvent::TextDelta("first".into()))];
    let client = client::ServerClient::with_token(&fixture.url, None, None).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let detach = tokio_util::sync::CancellationToken::new();
    let subscription = tokio::spawn(stream::subscribe(
        client,
        "agent-stream".into(),
        0,
        tx,
        detach.clone(),
    ));
    assert_eq!(rx.recv().await.unwrap().unwrap().seq, Some(1));
    fixture
        .state
        .lock()
        .unwrap()
        .events
        .push(frame(2, SessionEvent::TextDelta("second".into())));
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame.seq, Some(2));
    assert_eq!(frame.data["text"], "second");
    assert!(rx.try_recv().is_err());
    detach.cancel();
    subscription.await.unwrap();
}

#[test]
fn compacted_transcript_uses_display_hides_internal_context_and_preserves_usage() {
    let mut user = Message::user("u", "resolved prompt");
    user.display = Some("verbatim $skill @file".into());
    let mut internal = Message::user("summary", "INTERNAL SUMMARY");
    internal.synthetic = true;
    let mut assistant = Message::assistant("a");
    assistant.blocks.push(ContentBlock::text("answer"));
    assistant.usage.total_tokens = 456;
    let chat = transcript::replay(
        "operator:ops",
        &[user, internal, assistant],
        &[json!({"kind":"transcript_reset"})],
    );
    let text = chat
        .flatten()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("verbatim $skill @file"));
    assert!(text.contains("answer"));
    assert!(!text.contains("INTERNAL SUMMARY"));
    assert!(!text.contains("resolved prompt"));
    assert_eq!(chat.tokens_total, 456);
}

#[test]
fn snapshot_echo_uses_event_boundary_and_does_not_duplicate_a_later_admission() {
    let mut first = Message::user("first", "model preamble");
    first.display = Some("first input".into());
    let later = Message::user("later", "later input");
    let chat = transcript::replay(
        "agent:act",
        &[first, later],
        &[
            frame(
                1,
                SessionEvent::QueueConsumed {
                    seq: 1,
                    text: "first input".into(),
                },
            ),
            frame(2, SessionEvent::TextDelta("first answer".into())),
        ],
    );
    let text = chat
        .flatten()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("first input"));
    assert!(text.contains("first answer"));
    assert!(!text.contains("later input"));
    assert!(!text.contains("model preamble"));
}

#[tokio::test]
async fn remote_restore_recovers_after_a_transient_http_failure_without_local_execution() {
    let fixture = Fixture::new().await;
    let mut session = super::super::create(
        fixture.binding(ExecutionKind::Agent),
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap();
    session.harness.remote.as_mut().unwrap().created = true;
    fixture
        .store
        .set_harness_runtime(&session.id, &session.harness)
        .await
        .unwrap();
    fixture.state.lock().unwrap().auth_denied = true;
    let (commands, rx) = tokio::sync::mpsc::channel(16);
    let (ui, mut events) = tokio::sync::mpsc::channel(512);
    let worker = crate::worker::spawn_task(session, rx, ui);
    until(&mut events, |event| {
        matches!(event, UiEvent::Session(SessionEvent::Error(_)))
    })
    .await;
    fixture.state.lock().unwrap().auth_denied = false;
    until(&mut events, |event| {
        matches!(event, UiEvent::RemoteSnapshot { .. })
    })
    .await;
    assert!(fixture.mock.requests().is_empty());
    assert!(!fixture
        .state
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(|(path, _)| path == "POST /api/executions"));
    commands.send(UiCmd::Quit).await.unwrap();
    worker.await.unwrap();
}

#[tokio::test]
async fn queue_and_steer_admission_use_the_configured_proxy() {
    let fixture = Fixture::new().await;
    let mut binding = fixture.binding(ExecutionKind::Agent);
    binding.server_url = "http://fixture.invalid".into();
    binding.created = true;
    let session = super::super::create(
        binding,
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap();
    let input = opencoder_store::SessionInput {
        id: "proxied".into(),
        session_id: session.id,
        delivery: opencoder_store::Delivery::Queue,
        prompt: "via proxy".into(),
        images: vec![],
        seq: None,
        admitted_seq: 0,
        promoted_seq: None,
        display_text: None,
    };
    assert_eq!(
        super::super::admit(fixture.store.as_ref(), &input, Some(&fixture.url))
            .await
            .unwrap(),
        42
    );
    assert!(fixture
        .state
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(|(_, body)| body["action"] == "queue" && body["input"]["prompt"] == "via proxy"));
}

#[tokio::test]
async fn incomplete_remote_bookmark_never_falls_back_to_a_local_runner() {
    let fixture = Fixture::new().await;
    let session = super::super::create(
        fixture.binding(ExecutionKind::Agent),
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap();
    fixture
        .store
        .set_harness_runtime(
            &session.id,
            &opencoder_core::harness::HarnessRuntime::default(),
        )
        .await
        .unwrap();
    let result = super::super::load(
        &session.id,
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await;
    assert!(result.is_err());
    assert!(fixture.mock.requests().is_empty());
}

#[test]
fn compaction_resume_preserves_the_current_streaming_round_without_repeating_a_persisted_answer() {
    let user = Message::user("u", "prompt");
    let start = user.created_at + 1;
    let events = vec![
        frame(1, SessionEvent::TranscriptReset(vec![])),
        frame(
            2,
            SessionEvent::LlmRoundStart {
                started_at_ms: start,
            },
        ),
        frame(3, SessionEvent::TextDelta("partial answer".into())),
    ];
    let chat = transcript::replay("agent:act", std::slice::from_ref(&user), &events);
    let text = chat
        .flatten()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("partial answer"));
    let mut assistant = Message::assistant("a");
    assistant.created_at = start + 1;
    assistant
        .blocks
        .push(ContentBlock::text("partial answer complete"));
    let chat = transcript::replay("agent:act", &[user, assistant], &events);
    let text = chat
        .flatten()
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text.matches("partial answer").count(), 1);
}
