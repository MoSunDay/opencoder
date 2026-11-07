use opencoder_core::identity::{token_hash, Role};
use opencoder_store::{AccessToken, RdbStore, Store};

#[tokio::test]
async fn initial_token_failure_rolls_back_user_creation() {
    let store = RdbStore::open_memory().await.unwrap();
    let digest = token_hash("temporary-initial-token");
    store
        .create_user("alice", &digest, Role::Viewer, 1)
        .await
        .unwrap();
    assert!(store
        .create_user("bob", &digest, Role::Editor, 2)
        .await
        .is_err());
    assert!(store.find_user_by_name("bob").await.unwrap().is_none());
    assert_eq!(store.list_access_tokens().await.unwrap().len(), 1);
    assert_eq!(
        store
            .find_user_by_token_hash(&digest)
            .await
            .unwrap()
            .unwrap()
            .name,
        "alice"
    );
}

#[tokio::test]
async fn tokens_expire_individually_and_roles_are_read_live() {
    let store = RdbStore::open_memory().await.unwrap();
    store
        .create_user("reader", "", Role::Viewer, 1)
        .await
        .unwrap();
    let now = opencoder_core::message::now_ms();
    for (id, expires_at) in [
        ("forever", None),
        ("expired", Some(now - 1)),
        ("future", Some(now + 60_000)),
    ] {
        store
            .create_access_token(
                &AccessToken {
                    id: id.into(),
                    user_name: "reader".into(),
                    name: id.into(),
                    created_at: now,
                    expires_at,
                    revoked_at: None,
                },
                &token_hash(id),
            )
            .await
            .unwrap();
    }
    assert!(store
        .find_user_by_token_hash(&token_hash("expired"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .find_user_by_token_hash(&token_hash("future"))
        .await
        .unwrap()
        .is_some());
    assert!(store
        .update_user_role("reader", Role::Editor)
        .await
        .unwrap());
    assert_eq!(
        store
            .find_user_by_token_hash(&token_hash("forever"))
            .await
            .unwrap()
            .unwrap()
            .role,
        Role::Editor
    );
    assert!(store.revoke_access_token("future", now).await.unwrap());
    assert!(store.revoke_access_token("future", now + 1).await.unwrap());
    assert!(store
        .find_user_by_token_hash(&token_hash("future"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .find_user_by_token_hash(&token_hash("forever"))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .list_access_tokens()
            .await
            .unwrap()
            .iter()
            .find(|t| t.id == "future")
            .unwrap()
            .revoked_at,
        Some(now)
    );
    store.delete_user("reader").await.unwrap();
    assert!(store.list_access_tokens().await.unwrap().is_empty());
}
