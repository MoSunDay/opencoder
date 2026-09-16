use opencoder_brain::{PlaybookInput, Runtime};
use opencoder_llm::MockChatClient;
use opencoder_store::{BrainPlaybookRecord, LibsqlStore, Store};
use std::sync::Arc;
#[tokio::test]
async fn historical_playbooks_remain_read_only_and_unchanged() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let client = Arc::new(MockChatClient::new());
    let rt = Runtime::new(store.clone(), client, "test");
    let input: PlaybookInput =
        serde_json::from_value(serde_json::json!({"name":"legacy","steps":[]})).unwrap();
    let record=BrainPlaybookRecord{id:"legacy".into(),name:"legacy".into(),origin:"fixed".into(),situation_digest:None,spec_json:serde_json::json!({"schema_version":1,"id":"legacy","name":"legacy","origin":{"kind":"fixed"},"trigger":{"kind":"manual"},"steps":[]}).to_string(),created_at:1,updated_at:1};
    store.save_brain_playbook(&record).await.unwrap();
    assert!(rt
        .create_playbook(&input, 2)
        .await
        .unwrap_err()
        .to_string()
        .contains("migration"));
    assert!(rt.update_playbook("legacy", &input, 2).await.is_err());
    assert!(rt.delete_playbook("legacy").await.is_err());
    assert_eq!(
        rt.get_playbook("legacy").await.unwrap().unwrap().spec_json,
        record.spec_json
    );
    assert_eq!(rt.list_playbooks().await.unwrap().len(), 1);
}
