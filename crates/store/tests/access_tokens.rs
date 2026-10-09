use opencoder_core::identity::{token_hash, Role};
use opencoder_store::{AccessToken, LibsqlStore, Store};

#[tokio::test]
async fn tokens_expire_individually_and_roles_are_read_live() {
    let store = LibsqlStore::open_memory().await.unwrap();
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

#[tokio::test]
async fn admin_credential_is_preserved_when_users_and_tokens_are_separated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let digest = token_hash("temporary-test-admin");
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute("CREATE TABLE platform_users (name TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, role TEXT NOT NULL, created_at INTEGER NOT NULL)",()).await.unwrap();
    conn.execute(
        "INSERT INTO platform_users VALUES ('admin',?1,'admin',100)",
        libsql::params![digest.clone()],
    )
    .await
    .unwrap();
    conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
        .await
        .unwrap();
    conn.execute("INSERT INTO schema_version VALUES (34)", ())
        .await
        .unwrap();
    for role in ["user", "root"] {
        conn.execute(
            "INSERT INTO platform_users VALUES (?1,?2,?1,100)",
            libsql::params![role, token_hash(role)],
        )
        .await
        .unwrap();
    }
    drop(conn);
    drop(db);
    let store = LibsqlStore::open(&path).await.unwrap();
    let user = store
        .find_user_by_token_hash(&digest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.created_at, 100);
    let tokens = store.list_access_tokens().await.unwrap();
    assert_eq!(tokens.len(), 3);
    assert!(tokens
        .iter()
        .all(|token| token.expires_at.is_none() && token.revoked_at.is_none()));
    assert!(!store.revoke_access_token("user:admin", 200).await.unwrap());
    for role in ["user", "root"] {
        assert_eq!(
            store
                .find_user_by_token_hash(&token_hash(role))
                .await
                .unwrap()
                .unwrap()
                .role,
            Role::Viewer
        );
    }
    assert!(!store.update_user_role("admin", Role::Viewer).await.unwrap());
    drop(store);
    let reopened = LibsqlStore::open(&path).await.unwrap();
    assert!(reopened
        .find_user_by_token_hash(&digest)
        .await
        .unwrap()
        .is_some());
    assert_eq!(reopened.list_access_tokens().await.unwrap().len(), 3);
}
