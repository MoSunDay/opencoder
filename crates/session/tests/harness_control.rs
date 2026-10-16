#![cfg(unix)]
#[path = "harness/session.rs"]
mod fixtures;
use opencoder_core::harness::CodexSettings;
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{Delivery, SessionInput, Store};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn records(root: &Path, file: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join(file))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

async fn setup(root: &Path, mode: &str) -> (SessionState, Arc<dyn Store>) {
    let (mut session, store) = fixtures::session(root).await;
    let path = root.join("control.py");
    std::fs::write(&path, include_str!("harness/control_fixture.py")).unwrap();
    session.config.agent.codex = Some(CodexSettings {
        startup_script: vec!["/usr/bin/python3".into(), path.display().to_string()],
        ..Default::default()
    });
    session
        .harness
        .envs
        .insert("OPENCODER_EXTERNAL_INPUT_PROTOCOL".into(), "1".into());
    session
        .harness
        .envs
        .insert("CONTROL_TEST_MODE".into(), mode.into());
    session.turn_cancel = Some(Arc::new(Mutex::new(CancellationToken::new())));
    (session, store)
}

async fn started(path: &Path) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

fn input(id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.into(),
        session_id: "wrap-test".into(),
        delivery,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

async fn scenario(mode: &str, queue: bool) {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = setup(root.path(), mode).await;
    let token = session.turn_cancel.clone().unwrap();
    let marker = root.path().join("started");
    let writer = store.clone();
    let admission = tokio::spawn(async move {
        started(&marker).await;
        let request = input("stable-steer", Delivery::Steer, "correct current task");
        let first = writer.admit_input_once(&request).await.unwrap();
        let retry = writer.admit_input_once(&request).await.unwrap();
        assert_eq!(first.seq, retry.seq);
        assert!(first.inserted && !retry.inserted);
        if queue {
            writer
                .admit_input(&input("queue", Delivery::Queue, "queued task"))
                .await
                .unwrap();
        }
        token.lock().unwrap().cancel();
        first.seq
    });
    let mut events = vec![];
    tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "original task".into(), |e| events.push(e)),
    )
    .await
    .unwrap()
    .unwrap();
    let seq = admission.await.unwrap();
    let calls = records(root.path(), "normal.jsonl");
    assert_eq!(calls.len(), if queue { 2 } else { 1 });
    assert!(calls[0]["prompt"]
        .as_str()
        .unwrap()
        .ends_with("original task"));
    if queue {
        assert_eq!(calls[1]["prompt"], "queued task");
    }
    assert_eq!(
        records(root.path(), "finished.jsonl").len(),
        calls.len(),
        "observer must not be killed by steer"
    );
    let controls = records(root.path(), "controls.jsonl");
    assert_eq!(controls.len(), if mode == "pending_once" { 2 } else { 1 });
    assert!(controls
        .iter()
        .all(|c| c["parent_input_id"] == calls[0]["context"]["input_id"]));
    assert_eq!(records(root.path(), "effects.jsonl").len(), 1);
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|m| m.id == format!("external-input-{seq}"))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, SessionEvent::SteerConsumed { seq: s, .. } if *s == seq))
            .count(),
        1
    );
    assert!(store
        .pending_inputs(&session.id, Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
    if mode == "rejected" {
        assert!(session
            .messages
            .iter()
            .any(|m| m.text().contains("Input was not delivered")));
    }
}

#[tokio::test]
async fn steer_keeps_observer_and_queue_waits_for_next_turn() {
    scenario("accepted", true).await;
}
#[tokio::test]
async fn ended_turn_rejection_is_visible_and_never_becomes_followup() {
    scenario("rejected", false).await;
}
#[tokio::test]
async fn uncertain_control_reconciles_original_identity_without_duplicate_effect() {
    scenario("pending_once", false).await;
}

#[tokio::test]
async fn hard_cancel_uses_original_external_input_before_local_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, _) = setup(root.path(), "accepted").await;
    let token = CancellationToken::new();
    session.cancel = Some(token.clone());
    let marker = root.path().join("started");
    let cancel = tokio::spawn(async move {
        started(&marker).await;
        token.cancel();
    });
    tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "original task".into(), |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    cancel.await.unwrap();
    let normal = records(root.path(), "normal.jsonl");
    let controls = records(root.path(), "controls.jsonl");
    assert_eq!(normal.len(), 1);
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0]["action"], "cancel");
    assert_eq!(
        controls[0]["parent_input_id"],
        normal[0]["context"]["input_id"]
    );
    assert!(!session.harness.in_flight);
}

#[tokio::test]
async fn restart_reconciles_orphaned_control_without_resending_as_followup() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = setup(root.path(), "accepted").await;
    std::fs::write(root.path().join("released"), "initial completion").unwrap();
    run(&mut session, "original task".into(), |_| {})
        .await
        .unwrap();
    let parent = session.harness.last_input_id.clone().unwrap();
    let row = input("orphan", Delivery::Steer, "correction before restart");
    let seq = store.admit_input(&row).await.unwrap();
    store
        .promote_inputs(&session.id, i64::MAX, Delivery::Steer)
        .await
        .unwrap();
    let id = format!("external-input-{seq}");
    let journal = session
        .harness
        .resource_root
        .as_ref()
        .unwrap()
        .join("external-inputs");
    std::fs::create_dir_all(&journal).unwrap();
    std::fs::write(journal.join(format!("{seq}.request.json")), serde_json::json!({
        "schema_version":1, "action":"steer", "input_id":id, "parent_input_id":parent,
        "seq":seq, "original":row.prompt, "prompt":row.prompt, "images":[], "local_rejection":null
    }).to_string()).unwrap();
    // The external endpoint already applied this identity before its reply was lost.
    std::fs::write(root.path().join(format!("effect-{id}")), "once").unwrap();
    let mut resumed = opencoder_session::resume(
        store.clone(),
        &session.id,
        opencoder_core::Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    run(
        &mut resumed,
        "explicit request after restart".into(),
        |_| {},
    )
    .await
    .unwrap();
    let normal = records(root.path(), "normal.jsonl");
    assert_eq!(normal.len(), 2);
    assert_eq!(normal[1]["prompt"], "explicit request after restart");
    assert_eq!(records(root.path(), "controls.jsonl").len(), 1);
    assert!(
        !root.path().join("effects.jsonl").exists(),
        "reconciliation must not repeat the effect"
    );
    assert_eq!(resumed.messages.iter().filter(|m| m.id == id).count(), 1);
    assert_eq!(store.recover_orphan_inputs(&session.id).await.unwrap(), 0);
}

#[tokio::test]
async fn control_polling_does_not_extend_a_dead_observers_idle_timeout() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, _) = setup(root.path(), "accepted").await;
    session.config.stream_idle_timeout_secs = Some(2);
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        run(&mut session, "original task".into(), |_| {}),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("idle timeout"), "{error:#}");
    assert!(!root.path().join("controls.jsonl").exists());
    assert!(!session.harness.in_flight);
}
