//! Platform users and access tokens: unique constraints with rollback,
//! digest lookup with expiry/revocation, role updates and the last-admin
//! delete guard.

use super::*;

fn digest_of(text: &str) -> String {
    token_hash(text)
}

#[tokio::test]
async fn user_crud_roles_and_listing() {
    let store = mem().await;
    let created = store
        .create_user("alice", &digest_of("alice-token"), Role::Viewer, 10)
        .await
        .unwrap();
    assert_eq!(created.name, "alice");
    assert_eq!(created.role, Role::Viewer);
    assert_eq!(created.created_at, 10);

    // Same name conflicts with the SQL failure text.
    let err = store
        .create_user("alice", &digest_of("other"), Role::Editor, 11)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: platform_users.name"),
        "got: {err}"
    );

    store.create_user("bob", "", Role::Editor, 5).await.unwrap();
    let users = store.list_users().await.unwrap();
    assert_eq!(
        users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["bob", "alice"],
        "listing is created_at ASC, name ASC"
    );
    assert_eq!(store.count_admin_users().await.unwrap(), 0);

    // Roles: admin is the fixed startup credential — neither writable onto
    // a row nor rewritable on an admin row.
    assert!(store.update_user_role("alice", Role::Admin).await.is_err());
    assert!(store.update_user_role("alice", Role::Viewer).await.unwrap());
    assert!(!store.update_user_role("ghost", Role::Viewer).await.unwrap());
    assert_eq!(
        store
            .find_user_by_name("alice")
            .await
            .unwrap()
            .unwrap()
            .role,
        Role::Viewer
    );

    // Deleting a missing user is `false`; a real one is gone with its token.
    assert!(!store.delete_user("ghost").await.unwrap());
    assert!(store.delete_user("alice").await.unwrap());
    assert!(store.find_user_by_name("alice").await.unwrap().is_none());
    assert!(
        store
            .find_user_by_token_hash(&digest_of("alice-token"))
            .await
            .unwrap()
            .is_none(),
        "the user-delete cascade removes the token digest too"
    );
}

#[tokio::test]
async fn initial_token_failure_rolls_back_user_creation() {
    let store = mem().await;
    let digest = digest_of("temporary-initial-token");
    store
        .create_user("alice", &digest, Role::Viewer, 1)
        .await
        .unwrap();
    let err = store
        .create_user("bob", &digest, Role::Editor, 2)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: platform_tokens.token_hash"),
        "got: {err}"
    );
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

    // An empty digest creates the user without a token row.
    store
        .create_user("carol", "", Role::Viewer, 3)
        .await
        .unwrap();
    assert_eq!(store.list_access_tokens().await.unwrap().len(), 1);
}

#[tokio::test]
async fn token_hash_rotation_repoints_and_keeps_uniqueness() {
    let store = mem().await;
    store
        .create_user("admin", &digest_of("h1"), Role::Admin, 10)
        .await
        .unwrap();
    assert!(
        store
            .update_user_token_hash("admin", &digest_of("h2"))
            .await
            .unwrap(),
        "rotation repoints the digest"
    );
    assert!(store
        .find_user_by_token_hash(&digest_of("h1"))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .find_user_by_token_hash(&digest_of("h2"))
            .await
            .unwrap()
            .map(|u| u.name),
        Some("admin".into())
    );
    // Unknown names are a no-op; another user's digest is rejected.
    assert!(!store
        .update_user_token_hash("ghost", &digest_of("h3"))
        .await
        .unwrap());
    store
        .create_user("other", &digest_of("h4"), Role::Viewer, 11)
        .await
        .unwrap();
    assert!(
        store
            .update_user_token_hash("other", &digest_of("h2"))
            .await
            .is_err(),
        "rotating onto an existing digest must not be silently allowed"
    );
    // The rejected rotation left both digests where they were.
    assert!(store
        .find_user_by_token_hash(&digest_of("h4"))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .find_user_by_token_hash(&digest_of("h2"))
            .await
            .unwrap()
            .unwrap()
            .name,
        "admin"
    );
}

#[tokio::test]
async fn guarded_delete_never_removes_the_last_admin() {
    use opencoder_store::GuardedDelete;

    let store = mem().await;
    store
        .create_user("solo", &digest_of("solo"), Role::Admin, 10)
        .await
        .unwrap();
    store
        .create_user("plain", &digest_of("plain"), Role::Viewer, 11)
        .await
        .unwrap();
    assert_eq!(store.count_admin_users().await.unwrap(), 1);

    // The last admin survives; non-admins delete freely…
    assert_eq!(
        store.delete_user_guarding_last_admin("solo").await.unwrap(),
        GuardedDelete::LastAdmin
    );
    assert!(store.find_user_by_name("solo").await.unwrap().is_some());
    assert_eq!(
        store
            .delete_user_guarding_last_admin("plain")
            .await
            .unwrap(),
        GuardedDelete::Deleted
    );
    assert_eq!(
        store
            .delete_user_guarding_last_admin("plain")
            .await
            .unwrap(),
        GuardedDelete::Missing
    );
    // …and a second admin unlocks removal — the guard reads the count under
    // the same write lease the delete takes, so sequential deletes of both
    // admins stop at the survivor exactly like concurrent ones would.
    store
        .create_user("peer", &digest_of("peer"), Role::Admin, 12)
        .await
        .unwrap();
    assert_eq!(
        store.delete_user_guarding_last_admin("solo").await.unwrap(),
        GuardedDelete::Deleted
    );
    assert_eq!(
        store.delete_user_guarding_last_admin("peer").await.unwrap(),
        GuardedDelete::LastAdmin
    );
}

#[tokio::test]
async fn tokens_expire_individually_revoke_and_lookup_rules() {
    let store = mem().await;
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
                &opencoder_store::AccessToken {
                    id: id.into(),
                    user_name: "reader".into(),
                    name: id.into(),
                    created_at: now,
                    expires_at,
                    revoked_at: None,
                },
                &digest_of(id),
            )
            .await
            .unwrap();
    }
    assert!(store
        .find_user_by_token_hash(&digest_of("expired"))
        .await
        .unwrap()
        .is_none());
    assert!(store
        .find_user_by_token_hash(&digest_of("future"))
        .await
        .unwrap()
        .is_some());
    assert!(store
        .find_user_by_token_hash(&digest_of("forever"))
        .await
        .unwrap()
        .is_some());

    // Revocation removes the digest from auth immediately and is reported
    // as matched on repeat (COALESCE keeps the first stamp).
    assert!(store.revoke_access_token("forever", now + 1).await.unwrap());
    assert!(store
        .find_user_by_token_hash(&digest_of("forever"))
        .await
        .unwrap()
        .is_none());
    assert!(store.revoke_access_token("forever", now + 2).await.unwrap());
    assert!(!store.revoke_access_token("ghost", now).await.unwrap());

    let tokens = store.list_access_tokens().await.unwrap();
    assert_eq!(tokens.len(), 3, "revocation stamps, never deletes rows");
    assert_eq!(
        tokens
            .iter()
            .find(|t| t.id == "forever")
            .unwrap()
            .revoked_at,
        Some(now + 1)
    );

    // Admin-credential tokens are not revocable.
    store
        .create_user("root", &digest_of("root"), Role::Admin, 2)
        .await
        .unwrap();
    assert!(!store.revoke_access_token("user:root", now).await.unwrap());
    assert!(store
        .find_user_by_token_hash(&digest_of("root"))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn token_creation_checks_owner_and_unique_hash() {
    let store = mem().await;
    store
        .create_user("alice", &digest_of("alice"), Role::Viewer, 1)
        .await
        .unwrap();

    // The owner-user foreign key: no orphan queue rows for ghost users.
    let err = store
        .create_access_token(
            &opencoder_store::AccessToken {
                id: "t-ghost".into(),
                user_name: "ghost".into(),
                name: "n".into(),
                created_at: 1,
                expires_at: None,
                revoked_at: None,
            },
            &digest_of("unique-a"),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("FOREIGN KEY constraint failed"),
        "got: {err}"
    );

    // A duplicate digest rejects with the SQL conflict text.
    let err = store
        .create_access_token(
            &opencoder_store::AccessToken {
                id: "t-dup".into(),
                user_name: "alice".into(),
                name: "n".into(),
                created_at: 1,
                expires_at: None,
                revoked_at: None,
            },
            &digest_of("alice"),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: platform_tokens.token_hash"),
        "got: {err}"
    );
    assert_eq!(store.list_access_tokens().await.unwrap().len(), 1);
}
