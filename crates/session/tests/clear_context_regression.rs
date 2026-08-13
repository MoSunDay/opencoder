//! Regression test for `/act_clear_context <request>` losing the request when
//! the transcript holds a preserved plan (an assistant message). Previously the
//! preserved-plan branch unconditionally cleared `user_text`, dropping the
//! trailing request. This is a self-contained test file (does not depend on
//! helpers elsewhere) so it stays isolated from unrelated churn.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Regression: when the transcript holds an assistant message (so a plan is
/// preserved for handoff), a compound `/act_clear_context <request>` must NOT
/// discard the request. The request is recorded as a real user prompt and
/// executed alongside the plan handoff message.
#[tokio::test]
async fn clear_context_compound_keeps_rest_with_preserved_plan() {
    let store = mem_store().await;
    seed(&store, "clear-compound-plan", "act").await;

    // An assistant message makes final_plan_text() return Some, so the
    // preserved-plan branch is taken instead of the sentinel branch.
    let msgs = vec![Message::user("u1", "old question"), {
        let mut m = Message::assistant("a1");
        m.blocks.push(ContentBlock::text("I will implement X by..."));
        m
    }];
    store
        .append_messages("clear-compound-plan", &msgs)
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("fresh reply")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "clear-compound-plan",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();

    run(&mut session, "/act_clear_context review".into(), |_| {})
        .await
        .unwrap();

    // The request must be preserved (regression: it used to be discarded).
    let has_review = session
        .messages
        .iter()
        .any(|m| m.role == Role::User && m.text().contains("review") && !m.synthetic);
    assert!(
        has_review,
        "trailing arg 'review' must be recorded as a real user prompt"
    );

    // The LLM was called exactly once (plan handoff + request execution).
    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        1,
        "one LLM call to execute the preserved plan with the request"
    );

    // Both the preserved plan text and the request reach the model context.
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("I will implement X by..."),
        "preserved plan must appear in the model context: {body}"
    );
    assert!(
        body.contains("review"),
        "request 'review' must reach the model context: {body}"
    );
    assert!(
        !body.contains("/act_clear_context"),
        "raw command string must not reach the model: {body}"
    );

    // The execution produced an assistant turn with the reply text.
    let has_reply = session
        .messages
        .iter()
        .any(|m| m.role == Role::Assistant && m.text().contains("fresh reply"));
    assert!(has_reply, "execution reply recorded as an assistant turn");
}
