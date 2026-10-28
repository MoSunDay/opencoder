use super::*;
use crate::{
    chat::ChatBlock,
    worker::{UiCmd, UiEvent},
};
use opencoder_core::{fleet::ExecutionKind, Message};
use opencoder_session::SessionEvent;
use serde_json::json;
use support::{frame, until, Fixture};
mod protocol;
mod support;

#[tokio::test]
async fn remote_operator_creates_once_continues_same_execution_and_never_calls_local_model() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().events = vec![
        frame(1, SessionEvent::ReasoningDelta("think".into())),
        frame(
            2,
            SessionEvent::ToolStart {
                id: "c1".into(),
                name: "exec_command".into(),
                input: json!({"cmd":"pwd"}),
            },
        ),
        frame(
            3,
            SessionEvent::ToolEnd {
                id: "c1".into(),
                name: "exec_command".into(),
                output: "/workspace".into(),
                is_error: false,
                images: vec![],
            },
        ),
        frame(4, SessionEvent::TextDelta("**done**".into())),
        frame(5, SessionEvent::Done),
    ];
    let (id, tx, mut events, worker) = fixture.task(ExecutionKind::Operator).await;
    tx.send(UiCmd::Prompt(
        "read @notes.md $remote-skill".into(),
        vec!["data:image/png;base64,AA==".into()],
    ))
    .await
    .unwrap();
    let received = until(&mut events, |event| {
        matches!(event, UiEvent::Session(SessionEvent::Done))
    })
    .await;
    let mut chat = crate::chat::ChatView::default();
    for event in received {
        if let UiEvent::Session(event) = event {
            chat.apply(&event);
        }
    }
    assert!(chat
        .blocks
        .iter()
        .any(|block| matches!(block,ChatBlock::Assistant{raw,..} if raw=="**done**")));
    assert!(chat
        .blocks
        .iter()
        .any(|block| matches!(block,ChatBlock::StepGroup{steps,..} if !steps[0].calls.is_empty())));
    tx.send(UiCmd::Prompt("follow up".into(), vec![]))
        .await
        .unwrap();
    wait_for(&fixture, |requests| {
        requests
            .iter()
            .any(|(path, body)| path.ends_with("/commands") && body["action"] == "prompt")
    })
    .await;
    let requests = fixture.state.lock().unwrap().requests.clone();
    let creates: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "POST /api/executions")
        .collect();
    assert_eq!(creates.len(), 1);
    let request = &creates[0].1;
    assert_eq!(request["id"], id);
    assert_eq!(request["kind"], "operator");
    assert_eq!(request["target"], "codex");
    assert!(request["node_id"].is_null());
    assert!(request["input"].get("harness").is_none());
    assert_eq!(request["input"]["prompt"], "read @notes.md $remote-skill");
    assert_eq!(request["input"]["literal_mentions"], true);
    assert_eq!(request["input"]["images"][0], "data:image/png;base64,AA==");
    assert!(fixture.mock.requests().is_empty());
    tx.send(UiCmd::Quit).await.unwrap();
    worker.await.unwrap();
    assert!(!fixture
        .state
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(|(_, b)| b["action"] == "interrupt"));
}

#[tokio::test]
async fn failed_first_admission_retries_the_durable_request_with_the_same_id() {
    let fixture = Fixture::new().await;
    fixture.state.lock().unwrap().fail_create_once = true;
    let (id, tx, mut events, worker) = fixture.task(ExecutionKind::Agent).await;
    tx.send(UiCmd::Prompt("original".into(), vec![]))
        .await
        .unwrap();
    until(&mut events, |event| {
        matches!(event, UiEvent::Session(SessionEvent::Error(_)))
    })
    .await;
    assert!(
        !fixture
            .store
            .harness_runtime(&id)
            .await
            .unwrap()
            .unwrap()
            .remote
            .unwrap()
            .created
    );
    tx.send(UiCmd::Prompt("original".into(), vec![]))
        .await
        .unwrap();
    wait_for(&fixture, |requests| {
        requests
            .iter()
            .filter(|(p, _)| p == "POST /api/executions")
            .count()
            == 2
    })
    .await;
    tx.send(UiCmd::Quit).await.unwrap();
    worker.await.unwrap();
    let requests = fixture.state.lock().unwrap().requests.clone();
    let creates: Vec<_> = requests
        .iter()
        .filter(|(p, _)| p == "POST /api/executions")
        .map(|(_, b)| b)
        .collect();
    assert_eq!(creates[0], creates[1]);
    let binding = fixture
        .store
        .harness_runtime(&id)
        .await
        .unwrap()
        .unwrap()
        .remote
        .unwrap();
    assert!(binding.created);
    assert!(binding.initial_input.is_none());
}

#[tokio::test]
async fn remote_resume_replays_tools_and_user_boundaries_without_recreating_or_mixing_local_context(
) {
    let fixture = Fixture::new().await;
    let session = super::create(
        fixture.binding(ExecutionKind::Operator),
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap();
    let id = session.id;
    let mut runtime = fixture.store.harness_runtime(&id).await.unwrap().unwrap();
    runtime.remote.as_mut().unwrap().created = true;
    fixture
        .store
        .set_harness_runtime(&id, &runtime)
        .await
        .unwrap();
    {
        let mut state = fixture.state.lock().unwrap();
        let mut user = Message::user("u", "remote input");
        user.created_at = 1;
        state.messages.push(user);
        state.events = vec![
            frame(
                1,
                SessionEvent::ToolStart {
                    id: "call".into(),
                    name: "exec_command".into(),
                    input: json!({}),
                },
            ),
            frame(2, SessionEvent::TextDelta("remote output".into())),
        ];
    }
    let resumed = super::load(
        &id,
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(resumed.messages.is_empty());
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (ui, mut events) = tokio::sync::mpsc::channel(512);
    let worker = crate::worker::spawn_task(resumed, rx, ui);
    let received = until(&mut events, |event| {
        matches!(event, UiEvent::RemoteSnapshot { .. })
    })
    .await;
    let UiEvent::RemoteSnapshot { chat, running } = received.into_iter().last().unwrap() else {
        panic!()
    };
    assert!(running);
    assert!(chat.remote);
    assert_eq!(chat.agent, "operator:registered-codex");
    let text = chat
        .flatten()
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("remote input"));
    assert!(text.contains("remote output"));
    tx.send(UiCmd::Quit).await.unwrap();
    worker.await.unwrap();
    assert!(!fixture
        .state
        .lock()
        .unwrap()
        .requests
        .iter()
        .any(|(path, _)| path == "POST /api/executions"));
    let mut disabled = fixture.config();
    disabled.opencoder_server.enabled = false;
    assert!(super::load(
        &id,
        disabled,
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path()
    )
    .await
    .is_err());
}

#[tokio::test]
async fn remote_queue_steer_and_explicit_interrupt_target_server_not_local_store() {
    let fixture = Fixture::new().await;
    let session = super::create(
        fixture.binding(ExecutionKind::Agent),
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap();
    let id = session.id;
    let mut runtime = fixture.store.harness_runtime(&id).await.unwrap().unwrap();
    runtime.remote.as_mut().unwrap().created = true;
    fixture
        .store
        .set_harness_runtime(&id, &runtime)
        .await
        .unwrap();
    for delivery in [
        opencoder_store::Delivery::Queue,
        opencoder_store::Delivery::Steer,
    ] {
        let input = opencoder_store::SessionInput {
            id: format!("input-{delivery:?}"),
            session_id: id.clone(),
            delivery,
            prompt: "raw $skill @file".into(),
            images: vec![],
            seq: None,
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        };
        assert_eq!(
            super::admit(fixture.store.as_ref(), &input, None)
                .await
                .unwrap(),
            42
        );
        assert!(fixture
            .store
            .pending_inputs(&id, delivery)
            .await
            .unwrap()
            .is_empty());
    }
    let resumed = super::load(
        &id,
        fixture.config(),
        fixture.mock.clone(),
        fixture.store.clone(),
        fixture.workdir.path(),
    )
    .await
    .unwrap()
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (ui, mut events) = tokio::sync::mpsc::channel(512);
    let token = tokio_util::sync::CancellationToken::new();
    let worker = crate::worker::spawn_task(resumed.with_cancel(token.clone()), rx, ui);
    until(&mut events, |event| {
        matches!(event, UiEvent::RemoteSnapshot { .. })
    })
    .await;
    token.cancel();
    wait_for(&fixture, |requests| {
        requests
            .iter()
            .any(|(_, body)| body["action"] == "interrupt")
    })
    .await;
    tx.send(UiCmd::Quit).await.unwrap();
    worker.await.unwrap();
    let requests = fixture.state.lock().unwrap().requests.clone();
    for action in ["queue", "steer"] {
        assert!(requests
            .iter()
            .any(|(p, b)| p == &format!("POST /api/executions/{id}/commands")
                && b["action"] == action
                && b["input"]["prompt"] == "raw $skill @file"));
    }
}

async fn wait_for(fixture: &Fixture, predicate: impl Fn(&[(String, serde_json::Value)]) -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if predicate(&fixture.state.lock().unwrap().requests) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("request timeout");
}
