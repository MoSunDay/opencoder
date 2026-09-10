use super::wire::*;
use opencoder_core::{resolve_agent, ContentBlock, Message};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn coding_rounds_persist_replay_and_resume_with_full_provider_state() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    std::fs::write(
        dir.path().join("calc.py"),
        "assert 2 + 2 == 5\nprint('verified')\n",
    )
    .unwrap();
    let replies = vec![
        Reply::events(vec![completed(vec![
            reasoning(),
            call("r1", "read", json!({"path":"calc.py"})),
        ])]),
        Reply::events(vec![completed(vec![
            reasoning(),
            call(
                "e1",
                "edit",
                json!({"path":"calc.py","old_string":"== 5","new_string":"== 4"}),
            ),
        ])]),
        Reply::events(vec![completed(vec![
            reasoning(),
            call("b1", "bash", json!({"command":"python3 calc.py"})),
        ])]),
        Reply::events(vec![completed(vec![
            reasoning(),
            answer("Fixed and verified"),
        ])]),
        Reply::events(vec![completed(vec![answer("Resumed")])]),
    ];
    let server = serve(replies).await;
    let mut cfg = config(&server.url);
    cfg.interleaved_thinking = Some(false);
    let db = dir.path().join("session.db");
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(&db).await.unwrap());
    let mut session = SessionState::new(
        "responses-coding",
        resolve_agent("act").unwrap(),
        cfg.clone(),
        Arc::new(client(&cfg)),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    run(&mut session, "Fix calc.py and verify it".into(), |_| {})
        .await
        .unwrap();
    assert!(std::fs::read_to_string(dir.path().join("calc.py"))
        .unwrap()
        .contains("== 4"));
    assert!(session.messages.iter().flat_map(|m|&m.blocks).any(|b|matches!(b,ContentBlock::ToolResult{tool_use_id,content,is_error:false,..}if tool_use_id=="b1" && content.contains("verified"))));
    let saved = store.load_messages("responses-coding").await.unwrap();
    assert_eq!(
        saved.iter().filter(|m| m.provider_state.is_some()).count(),
        4
    );
    assert!(saved
        .iter()
        .filter(|m| m.provider_state.is_some())
        .all(|m| m
            .blocks
            .iter()
            .any(|b| matches!(b,ContentBlock::Reasoning{text} if text=="plan"))));
    assert!(saved
        .iter()
        .filter(|m| m.provider_state.is_some())
        .all(|m| m.usage.reasoning_tokens == 20));
    {
        let requests = server.requests.lock().unwrap();
        let input = requests[1].1["input"].as_array().unwrap();
        let pos = input.iter().position(|v| v["type"] == "reasoning").unwrap();
        assert_eq!(input[pos]["encrypted_content"], "opaque-fixture");
        assert_eq!(input[pos + 1]["call_id"], "r1");
        assert_eq!(input[pos + 2]["type"], "function_call_output");
        assert_eq!(input[pos + 2]["call_id"], "r1");
    }
    let fork = opencoder_session::fork::fork_session(store.as_ref(), "responses-coding")
        .await
        .unwrap();
    let forked = store.load_messages(&fork).await.unwrap();
    assert_eq!(
        serde_json::to_value(&forked).unwrap(),
        serde_json::to_value(&saved).unwrap()
    );
    drop(session);
    drop(store);
    let reopened: Arc<dyn Store> = Arc::new(LibsqlStore::open(&db).await.unwrap());
    let mut resumed = opencoder_session::resume::resume(
        reopened,
        "responses-coding",
        cfg.clone(),
        Arc::new(client(&cfg)),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();
    run(&mut resumed, "Continue".into(), |_| {}).await.unwrap();
    let requests = server.requests.lock().unwrap();
    let input = requests[4].1["input"].as_array().unwrap();
    assert_eq!(input.iter().filter(|v| v["type"] == "reasoning").count(), 4);
    assert!(input.iter().any(|v| v["phase"] == "final_answer"));
}

#[tokio::test]
async fn compaction_preserves_complete_tool_groups_and_replays_retained_state() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    let server = serve(vec![
        Reply::events(vec![completed(vec![answer("Summary")])]),
        Reply::events(vec![completed(vec![answer("Continued")])]),
    ])
    .await;
    let mut cfg = config(&server.url);
    cfg.compaction.tail_turns = 1;
    let mut session = SessionState::new(
        "compact",
        resolve_agent("act").unwrap(),
        cfg.clone(),
        Arc::new(client(&cfg)),
        dir.path().to_path_buf(),
    );
    session.messages.push(Message::user("old", "previous task"));
    let mut old = Message::assistant("old-a");
    old.blocks.push(ContentBlock::text("previous answer"));
    session.messages.push(old);
    session
        .messages
        .push(Message::user("recent", "current task"));
    let mut assistant = Message::assistant("call");
    assistant.blocks.push(ContentBlock::ToolUse {
        id: "c1".into(),
        name: "read".into(),
        input: json!({"path":"a"}),
    });
    assistant.provider_state = Some(opencoder_core::ProviderState {
        provider: "fixture".into(),
        base_url: server.url.clone(),
        model: "gpt-6-astra".into(),
        output: vec![reasoning(), call("c1", "read", json!({"path":"a"}))],
    });
    session.messages.push(assistant);
    let mut result = Message::user("result", "");
    result.blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "c1".into(),
        content: "contents".into(),
        is_error: false,
        images: vec![],
    }];
    session.messages.push(result);
    assert_eq!(
        opencoder_session::compaction::compact(
            &mut session,
            &opencoder_session::tools::registry(),
            &mut |_| {}
        )
        .await
        .unwrap(),
        Some("Summary".into())
    );
    assert!(session.messages.iter().any(|m| m.provider_state.is_some()));
    run(&mut session, "continue".into(), |_| {}).await.unwrap();
    let requests = server.requests.lock().unwrap();
    let body = &requests[1].1;
    let input = body["input"].as_array().unwrap();
    assert!(input.iter().any(|v| v["type"] == "reasoning"));
    assert!(input
        .iter()
        .any(|v| v["type"] == "function_call" && v["call_id"] == "c1"));
    assert!(input
        .iter()
        .any(|v| v["type"] == "function_call_output" && v["call_id"] == "c1"));
}
