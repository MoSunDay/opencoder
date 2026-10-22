use opencoder_core::identity::{Identity, Role};
use opencoder_store::project::comments::{CommentError, CommentMutation, CommentQuery, NewComment};
use opencoder_store::{ProjectStore, TursoStore};
use serde_json::json;

async fn setup() -> (TursoStore, NewComment, i64) {
    let store = TursoStore::open_memory().await.unwrap();
    for id in ["task", "other"] {
        store.create_todo(&serde_json::from_value(json!({"id":id,"title":"Task","draft":"body","status":"draft","agent":"act","created_at":1,"updated_at":1})).unwrap()).await.unwrap();
    }
    let note = NewComment {
        todo_id: "task".into(),
        request_id: "initial".into(),
        body_md: "original".into(),
        author_kind: "capability".into(),
        author_id: "capability".into(),
        author_name: "name".into(),
        capability_kind: Some("operator".into()),
        submitted_by: "alice".into(),
        execution_id: None,
        created_at: 2,
    };
    let seq = store.append_todo_comment(&note).await.unwrap().seq;
    (store, note, seq)
}

fn change(seq: i64, body: Option<&str>) -> CommentMutation {
    CommentMutation {
        todo_id: "task".into(),
        seq,
        expected_revision: 1,
        body_md: body.map(str::to_owned),
        actor: Identity {
            name: "alice".into(),
            role: Role::Editor,
        },
        now: 10,
    }
}

#[tokio::test]
async fn edit_preserves_identity_and_creation_retries_return_current_body() {
    let (store, original, seq) = setup().await;
    let updated = store
        .mutate_todo_comment(&change(seq, Some("**merged**")))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.seq, seq);
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.updated_at, 10);
    let mut expected = original.clone();
    expected.body_md = "**merged**".into();
    assert_eq!(updated.entry, expected);
    let retried = store.append_todo_comment(&original).await.unwrap();
    assert_eq!(retried.entry.body_md, "**merged**");
    assert_eq!(retried.revision, 2);
    let value = serde_json::to_value(&retried).unwrap();
    assert!(value.get("original_body_md").is_none());
    assert_eq!(
        store.get_todo("task").await.unwrap().unwrap().updated_at,
        10
    );
    assert_eq!(
        store.get_todo("task").await.unwrap().unwrap().board_status,
        "backlog"
    );
    let page = store
        .todo_comments("task", CommentQuery::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.comments.len(), 1);
    assert_eq!(page.comments[0].revision, 2);
    assert_eq!(page.comments[0].entry.body_md, "**merged**");
}

#[tokio::test]
async fn concurrent_edits_reject_stale_writes_and_deletes() {
    let (store, _, seq) = setup().await;
    let a = change(seq, Some("first"));
    let b = change(seq, Some("second"));
    let (a, b) = tokio::join!(store.mutate_todo_comment(&a), store.mutate_todo_comment(&b));
    assert_ne!(a.is_ok(), b.is_ok());
    let failed = a.err().or_else(|| b.err()).unwrap();
    assert!(matches!(
        failed.downcast_ref(),
        Some(CommentError::RevisionConflict)
    ));
    assert!(matches!(
        store
            .mutate_todo_comment(&change(seq, None))
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::RevisionConflict)
    ));
    assert_eq!(
        store
            .todo_comments("task", CommentQuery::default())
            .await
            .unwrap()
            .unwrap()
            .comments
            .len(),
        1
    );
}

#[tokio::test]
async fn delete_is_repeatable_but_old_append_requests_cannot_resurrect_it() {
    let (store, original, seq) = setup().await;
    let delete = change(seq, None);
    assert!(store.mutate_todo_comment(&delete).await.unwrap().is_none());
    assert!(store.mutate_todo_comment(&delete).await.unwrap().is_none());
    assert!(store
        .todo_comments("task", CommentQuery::default())
        .await
        .unwrap()
        .unwrap()
        .comments
        .is_empty());
    assert!(matches!(
        store
            .append_todo_comment(&original)
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::Deleted)
    ));
    assert!(matches!(
        store
            .mutate_todo_comment(&change(seq, Some("restore")))
            .await
            .unwrap_err()
            .downcast_ref(),
        Some(CommentError::MissingComment)
    ));
    store.delete_todo("task").await.unwrap();
    assert!(store
        .todo_comment_request("task", "alice", "initial")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn mutations_enforce_author_ownership_task_scope_and_content_limits() {
    let (store, _, seq) = setup().await;
    for body in [Some("new"), None] {
        for actor in [
            Identity {
                name: "bob".into(),
                role: Role::Editor,
            },
            Identity {
                name: "alice".into(),
                role: Role::Viewer,
            },
        ] {
            let mut input = change(seq, body);
            input.actor = actor;
            assert!(matches!(
                store
                    .mutate_todo_comment(&input)
                    .await
                    .unwrap_err()
                    .downcast_ref(),
                Some(CommentError::Forbidden)
            ));
        }
        let mut input = change(seq, body);
        input.todo_id = "other".into();
        assert!(matches!(
            store
                .mutate_todo_comment(&input)
                .await
                .unwrap_err()
                .downcast_ref(),
            Some(CommentError::MissingComment)
        ));
    }
    for body in [" ".to_owned(), "x".repeat(65537)] {
        assert!(matches!(
            store
                .mutate_todo_comment(&change(seq, Some(&body)))
                .await
                .unwrap_err()
                .downcast_ref(),
            Some(CommentError::InvalidInput)
        ));
    }
    let mut input = change(seq, Some("new"));
    input.actor = Identity::admin("admin");
    assert_eq!(
        store
            .mutate_todo_comment(&input)
            .await
            .unwrap()
            .unwrap()
            .entry
            .submitted_by,
        "alice"
    );
}

#[tokio::test]
async fn unmodified_comments_gain_revisions_without_rewriting_content() {
    let (store, note, seq) = setup().await;
    let mut unchanged = change(seq, Some(&note.body_md));
    unchanged.now = 99;
    let result = store
        .mutate_todo_comment(&unchanged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.revision, 1);
    assert_eq!(result.updated_at, 2);
    assert_eq!(result.seq, seq);
    assert_eq!(store.get_todo("task").await.unwrap().unwrap().updated_at, 2);
}
