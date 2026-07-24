//! Regression: plan->act handoff must not duplicate data in the store.
//! Covers handoff -> run("") -> resume -> run, with steers.
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{LlmEvent, MockChatClient, Usage};
use opencoder_session::{plan_handoff, run, resume, SessionState};
use opencoder_store::{LibsqlStore, SessionPatch, Store};

fn config() -> Config {
    Config { model: "m/g".into(), ..Config::default() }
}

fn done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage { input_tokens: 5, output_tokens: 1, total_tokens: 6, ..Default::default() }),
    }
}

fn assert_no_dup_ids(msgs: &[Message]) {
    let mut ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "DUPLICATE ids: {:#?}", msgs.iter().map(|m| (&m.id, m.role, m.text())).collect::<Vec<_>>());
}

fn mk_plan_session(store: Arc<dyn Store>, id: &str) -> SessionState {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done("## Plan\n1. do X")]));
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("plan").unwrap();
    let mut s = SessionState::new(id, agent, config(), mock, dir.path().to_path_buf());
    s.store = Some(store);
    s
}

#[tokio::test]
async fn handoff_run_no_duplicate() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let _dir = tempfile::tempdir().unwrap();
    let mut session = mk_plan_session(store.clone(), "dup-1");
    run(&mut session, "plan something".into(), |_| {}).await.unwrap();
    assert_eq!(store.load_messages(&session.id).await.unwrap().len(), 2);

    session.agent = resolve_agent("act").unwrap();
    assert!(plan_handoff::handoff(&mut session, "").is_some());
    store.update_session(&session.id, &SessionPatch {
        handoff_seq: session.handoff_seq, handoff_plan: session.handoff_plan.clone(), ..Default::default()
    }).await.unwrap();

    session.client = Arc::new(MockChatClient::new().push_script(vec![done("Done!")]));
    run(&mut session, String::new(), |_| {}).await.unwrap();

    let msgs = store.load_messages(&session.id).await.unwrap();
    assert_eq!(msgs.len(), 3, "got: {:#?}", msgs.iter().map(|m| (&m.id, m.text())).collect::<Vec<_>>());
    assert_no_dup_ids(&msgs);
}

#[tokio::test]
async fn handoff_resume_run_no_duplicate() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = mk_plan_session(store.clone(), "dup-2");
    run(&mut session, "plan something".into(), |_| {}).await.unwrap();

    session.agent = resolve_agent("act").unwrap();
    plan_handoff::handoff(&mut session, "");
    store.update_session(&session.id, &SessionPatch {
        handoff_seq: session.handoff_seq, handoff_plan: session.handoff_plan.clone(), ..Default::default()
    }).await.unwrap();

    session.client = Arc::new(MockChatClient::new().push_script(vec![done("Step1")]));
    run(&mut session, String::new(), |_| {}).await.unwrap();
    let n_before = store.load_messages(&session.id).await.unwrap().len();

    // resume
    let mock2 = Arc::new(MockChatClient::new().push_script(vec![done("Step2")]));
    let mut resumed = resume(store.clone(), &session.id, config(), mock2, dir.path().to_path_buf()).await.unwrap();
    let n_after_resume = store.load_messages(&session.id).await.unwrap().len();
    assert_eq!(n_after_resume, n_before, "resume must not add messages");

    run(&mut resumed, "continue".into(), |_| {}).await.unwrap();
    let final_msgs = store.load_messages(&session.id).await.unwrap();
    assert_eq!(final_msgs.len(), n_before + 2, "got: {:#?}", final_msgs.iter().map(|m| (&m.id, m.text())).collect::<Vec<_>>());
    assert_no_dup_ids(&final_msgs);
}

#[tokio::test]
async fn handoff_steer_consumed_once() {
    use opencoder_store::{Delivery, SessionInput};
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let _dir = tempfile::tempdir().unwrap();
    let mut session = mk_plan_session(store.clone(), "dup-3");
    run(&mut session, "plan something".into(), |_| {}).await.unwrap();

    // admit a steer while plan agent is idle (un-promoted)
    store.admit_input(&SessionInput {
        seq: None, id: "in1".into(), session_id: session.id.clone(),
        delivery: Delivery::Steer, prompt: "look at module X".into(),
        images: Vec::new(),
        admitted_seq: 1, promoted_seq: None,
    }).await.unwrap();

    session.agent = resolve_agent("act").unwrap();
    plan_handoff::handoff(&mut session, "");
    store.update_session(&session.id, &SessionPatch {
        handoff_seq: session.handoff_seq, handoff_plan: session.handoff_plan.clone(), ..Default::default()
    }).await.unwrap();

    // act run("") -> claim_steers should promote the steer exactly once
    session.client = Arc::new(MockChatClient::new()
        .push_script(vec![done("considering steer")])
        .push_script(vec![done("Done!")]));
    run(&mut session, String::new(), |_| {}).await.unwrap();

    let msgs = store.load_messages(&session.id).await.unwrap();
    // plan user(1) + plan assistant(1) + steer-as-user(1) + act assistant(1) + act assistant(1) = 5
    let steer_count = msgs.iter().filter(|m| m.text().contains("look at module X")).count();
    assert_eq!(steer_count, 1, "steer must appear exactly once, got {steer_count}: {:#?}", msgs.iter().map(|m| m.text()).collect::<Vec<_>>());
    assert_no_dup_ids(&msgs);
}
