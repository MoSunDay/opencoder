//! Message persistence contracts: roles/blocks/usage round-trip and JSONL
//! directory import.

use crate::common::{conv, fresh, make_session};
use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, Store};

#[tokio::test]
async fn append_and_load_preserves_all_roles_and_blocks() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1).await;

    let original = vec![
        Message::user("u1", "hello"),
        {
            let mut m = Message::assistant("a1");
            m.blocks = vec![
                ContentBlock::text("I will use a tool"),
                ContentBlock::ToolUse {
                    id: "tu1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ];
            m.agent = Some("act".into());
            m.model = Some("glm-5.2".into());
            m.usage = opencoder_core::MessageUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cache_read_tokens: 104_857_600,
                cache_creation_tokens: 1_500,
            };
            m.created_at = 2;
            m
        },
        {
            let id = "t1";
            Message {
                id: id.into(),
                role: Role::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id: "tu1".into(),
                    content: "file.txt".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
                model: None,
                agent: None,
                usage: Default::default(),
                created_at: 3,
                synthetic: false,
            }
        },
    ];

    let seqs = store.append_messages("s1", &original).await.unwrap();
    assert_eq!(seqs.len(), 3);
    assert_eq!(seqs, vec![1, 2, 3]);

    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].role, Role::User);
    assert_eq!(loaded[0].text(), "hello");
    assert_eq!(loaded[1].role, Role::Assistant);
    assert_eq!(loaded[1].agent.as_deref(), Some("act"));
    assert_eq!(loaded[1].model.as_deref(), Some("glm-5.2"));
    assert_eq!(loaded[1].usage.total_tokens, 15);
    // Cache tokens must survive the SQLite JSON round-trip (usage_json column).
    assert_eq!(loaded[1].usage.cache_read_tokens, 104_857_600);
    assert_eq!(loaded[1].usage.cache_creation_tokens, 1_500);
    assert_eq!(loaded[1].blocks.len(), 2);
    match &loaded[1].blocks[1] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "tu1");
            assert_eq!(name, "bash");
            assert_eq!(input["command"], "ls");
        }
        other => panic!("expected ToolUse, got {other:?}"),
    }
    assert_eq!(loaded[2].role, Role::Tool);
}

#[tokio::test]
async fn jsonl_import_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let jsonl_dir = dir.path().join("sessions");
    tokio::fs::create_dir_all(&jsonl_dir).await.unwrap();

    let original: Vec<Message> = conv("imp", 4);
    let path = jsonl_dir.join("imp-session.jsonl");
    let mut text = String::new();
    for m in &original {
        text.push_str(&serde_json::to_string(m).unwrap());
        text.push('\n');
    }
    tokio::fs::write(&path, text).await.unwrap();

    let db = dir.path().join("imp.db");
    let store = LibsqlStore::open(&db).await.unwrap();
    let report = opencoder_store::import::import_jsonl_dir(&store, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report.sessions, 1);
    assert_eq!(report.messages, 4);

    let loaded = store.load_messages("imp-session").await.unwrap();
    assert_eq!(loaded.len(), original.len());
    for (a, b) in original.iter().zip(loaded.iter()) {
        assert_eq!(a.role, b.role, "role mismatch");
        assert_eq!(a.text(), b.text(), "text mismatch");
        assert_eq!(a.created_at, b.created_at, "ts mismatch");
    }

    // idempotent re-run: skips already-imported
    let report2 = opencoder_store::import::import_jsonl_dir(&store, &jsonl_dir)
        .await
        .unwrap();
    assert_eq!(report2.sessions, 0, "second run skips existing");
}

