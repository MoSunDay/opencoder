use opencoder_core::{Config, Role};
use opencoder_llm::{LlmEvent, MockChatClient, RequestPurpose};
use opencoder_session::{resume, run};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use std::sync::Arc;

fn mock() -> Arc<MockChatClient> {
    Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "verified result".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    )
}

async fn seed(store: &dyn Store, kind: Option<&str>) {
    store
        .create_session(&SessionMeta {
            id: "prompt-test".into(),
            kind: kind.map(str::to_owned),
            agent: Some("act".into()),
            title: Some("Prompt test".into()),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn operator_defaults_survive_compaction_context_clear_and_resume() {
    let root = tempfile::tempdir().unwrap();
    let skills = root.path().join("execution home/.opencoder/skills");
    std::fs::create_dir_all(skills.join("focused")).unwrap();
    std::fs::write(
        skills.join("focused/SKILL.md"),
        "# Focused\nOnly inspect this input.",
    )
    .unwrap();
    opencoder_core::skill::with_execution(Some(skills.clone()), async {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        seed(store.as_ref(), Some("operator")).await;
        let mock = mock();
        let mut config = Config {
            model: "mock/model".into(),
            ..Config::default()
        };
        config.compaction.tail_turns = 1;
        let mut session = resume(
            store.clone(),
            "prompt-test",
            config.clone(),
            mock.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut session, "inspect one item".into(), |_| {})
            .await
            .unwrap();
        run(&mut session, "verify the same item".into(), |_| {})
            .await
            .unwrap();
        assert!(opencoder_session::compaction::compact(
            &mut session,
            &Default::default(),
            &mut |_| {},
        )
        .await
        .unwrap()
        .is_some());
        run(&mut session, "continue after compaction".into(), |_| {})
            .await
            .unwrap();
        run(&mut session, "/act_clear_context".into(), |_| {})
            .await
            .unwrap();
        let mut restored = resume(
            store.clone(),
            "prompt-test",
            config,
            mock.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut restored, "report the result".into(), |_| {})
            .await
            .unwrap();
        let requests = mock.requests();
        let conversations: Vec<_> = requests
            .iter()
            .filter(|request| request.purpose == RequestPurpose::Conversation)
            .collect();
        assert!(conversations.len() >= 4);
        for request in conversations {
            let system = &request.messages[0];
            assert_eq!(system.role, Role::System);
            let text = system.text();
            assert_eq!(text.matches("## Operator 默认规则").count(), 1);
            assert!(text.contains(skills.to_str().unwrap()));
            assert!(text.contains("只负责分配给你的任务和结果"));
            assert!(
                !text.contains("Only inspect this input."),
                "skill bodies load on demand"
            );
        }
        assert!(requests
            .last()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.text() == "report the result"));
        assert!(!store
            .load_messages("prompt-test")
            .await
            .unwrap()
            .iter()
            .any(|m| m.text().contains("Operator 默认规则")));
    })
    .await;
}

#[tokio::test]
async fn other_session_kinds_do_not_receive_operator_defaults() {
    let root = tempfile::tempdir().unwrap();
    for kind in [None, Some("agent"), Some("maintenance"), Some("brain")] {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        seed(store.as_ref(), kind).await;
        let mock = mock();
        let mut session = resume(
            store,
            "prompt-test",
            Config::default(),
            mock.clone(),
            root.path().into(),
        )
        .await
        .unwrap();
        run(&mut session, "inspect one item".into(), |_| {})
            .await
            .unwrap();
        assert_eq!(mock.requests().len(), 1);
        assert!(
            !mock.requests()[0].messages[0]
                .text()
                .contains("Operator 默认规则"),
            "{kind:?}"
        );
    }
}
