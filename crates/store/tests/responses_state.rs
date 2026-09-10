use opencoder_core::{ContentBlock, Message, ProviderState};
use opencoder_store::{bundle, LibsqlStore, SessionMeta, Store};
use serde_json::json;

#[tokio::test]
async fn legacy_v23_migration_preserves_messages_and_bundle_roundtrips_responses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    // Construct an actual v23 messages table; no existing database is modified.
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_version(version INTEGER NOT NULL);
        INSERT INTO schema_version VALUES(23);
        CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,
        session_id TEXT NOT NULL, role TEXT NOT NULL, agent TEXT, model TEXT,
        blocks_json TEXT NOT NULL, usage_json TEXT NOT NULL, created_at INTEGER NOT NULL,
        synthetic INTEGER NOT NULL DEFAULT 0, display TEXT, mode TEXT,
        summary INTEGER NOT NULL DEFAULT 0);
        INSERT INTO messages(id,session_id,role,blocks_json,usage_json,created_at)
        VALUES('old','s','user','[{\"kind\":\"text\",\"text\":\"keep me\"}]','{\"input_tokens\":1,\"output_tokens\":0,\"total_tokens\":1}',1);",
    )
    .await
    .unwrap();
    drop(conn);
    drop(db);
    let store = LibsqlStore::open(&path).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "s".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let old = store.load_messages("s").await.unwrap();
    assert_eq!(old[0].text(), "keep me");
    assert!(old[0].provider_state.is_none());
    assert_eq!(old[0].usage.reasoning_tokens, 0);
    assert_eq!(old[0].usage.input_tokens, 1);
    let mut message = Message::assistant("new");
    message.blocks.push(ContentBlock::text("done"));
    message.usage.reasoning_tokens = 42;
    message.provider_state = Some(ProviderState {
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        model: "gpt-6".into(),
        output: vec![
            json!({"type":"reasoning", "encrypted_content":"opaque-fixture", "summary":[]}),
            json!({"type":"message", "phase":"final_answer", "content":[{"type":"output_text","text":"done"}]}),
        ],
    });
    store.append_message("s", &message).await.unwrap();
    let exported = bundle::export_bundle(&store, "s").await.unwrap();
    let mut bytes = Vec::new();
    bundle::write_bundle(&exported, &mut bytes).unwrap();
    let decoded = bundle::read_bundle(&mut bytes.as_slice()).unwrap();
    let imported_id = bundle::import_bundle(&store, &decoded, None).await.unwrap();
    let imported = store.load_messages(&imported_id).await.unwrap();
    assert_eq!(imported[1].provider_state, message.provider_state);
    assert_eq!(imported[1].usage.reasoning_tokens, 42);
    drop(store);
    let reopened = LibsqlStore::open(&path).await.unwrap();
    assert_eq!(
        reopened.load_messages("s").await.unwrap()[1].provider_state,
        message.provider_state
    );
}
