use opencoder_brain::Runtime;
use opencoder_llm::MockChatClient;
use opencoder_store::LibsqlStore;
use std::sync::Arc;
#[tokio::test]
async fn retired_planners_and_dispatchers_reject_without_model_or_store_mutation() {
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let client = Arc::new(MockChatClient::new());
    let rt = Runtime::new(store, client.clone(), "test");
    for result in [
        rt.plan_decision_tree("m", "s", 1, 1).await.map(|_| ()),
        rt.dispatch_decision_tree("p", "s").await.map(|_| ()),
        rt.dispatch_or_plan("m", "s", 1, false, 1).await.map(|_| ()),
        rt.plan_playbook("m", "s", 1, false, 1).await.map(|_| ()),
    ] {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("migration required"));
    }
    assert_eq!(client.call_count(), 0);
}
