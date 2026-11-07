//! Execution-inventory lanes: full-snapshot shape, tracked-change staging,
//! lane filtering, removals, bounded reads that never need message previews
//! or project bodies, and subagent membership. Ported from
//! `tests/inventory_sources.rs`.

use super::inventory_fixtures::{run, session, subagent, todo};
use super::*;
use opencoder_core::Message;
use opencoder_store::{ProjectStore, ProjectTodoRunPatch, ProjectTodoRunStatus, SessionPatch};

#[tokio::test]
async fn inventory_tracks_tracked_changes_lane_filtering_and_deletion() {
    let store = mem().await;
    // Empty store: a full snapshot with nothing in it and revision 0.
    let empty = store.inventory_sources(None).await.unwrap();
    assert!(empty.full);
    assert_eq!(empty.revision, 0);
    assert!(empty.sessions.is_empty() && empty.project_runs.is_empty());
    assert!(empty.removed_sessions.is_empty() && empty.removed_runs.is_empty());

    store
        .create_session(&session("agent-test", None, 1))
        .await
        .unwrap();
    let full = store.inventory_sources(None).await.unwrap();
    assert!(full.full);
    assert_eq!(full.revision, 1);
    assert_eq!(full.sessions.len(), 1);
    assert_eq!(full.sessions[0].id, "agent-test");
    assert_eq!(full.sessions[0].kind, None);
    assert_eq!(full.sessions[0].created_at, 1);

    // `updated_at`/`summary` are not tracked columns: no change is staged,
    // so an up-to-date cursor sees an empty delta at the same revision.
    store
        .update_session(
            "agent-test",
            &SessionPatch {
                updated_at: Some(2),
                summary: Some("ignored".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let idle = store.inventory_sources(Some(full.revision)).await.unwrap();
    assert_eq!(idle.revision, full.revision);
    assert!(!idle.full);
    assert!(idle.sessions.is_empty());
    assert!(idle.removed_sessions.is_empty());

    // A title change is tracked: the cursor advances, the current row comes
    // back and the same id is also reported as a removal (re-add semantics).
    store
        .update_session(
            "agent-test",
            &SessionPatch {
                title: Some("renamed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let retitled = store.inventory_sources(Some(idle.revision)).await.unwrap();
    assert!(retitled.revision > idle.revision, "cursor is monotonic");
    assert_eq!(retitled.sessions.len(), 1);
    assert_eq!(retitled.sessions[0].title.as_deref(), Some("renamed"));
    assert_eq!(retitled.removed_sessions, vec!["agent-test"]);

    // Operator-lane rows never enter the execution inventory …
    store
        .create_session(&session("operator-lane", Some("operator"), 2))
        .await
        .unwrap();
    let with_operator = store.inventory_sources(None).await.unwrap();
    assert!(with_operator.full);
    assert_eq!(
        with_operator
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-test"]
    );
    // … and a staged change for one reports only a removal.
    store.delete_session("operator-lane").await.unwrap();
    let hidden = store
        .inventory_sources(Some(with_operator.revision))
        .await
        .unwrap();
    assert!(hidden.sessions.is_empty());
    assert_eq!(hidden.removed_sessions, vec!["operator-lane"]);
    assert!(hidden.revision > with_operator.revision);

    // The delta starts strictly after the cursor (`seq > ?1` in the SQL
    // reader): the operator removal the caller already consumed at
    // `hidden.revision` is not reported a second time.
    store.delete_session("agent-test").await.unwrap();
    let removed = store
        .inventory_sources(Some(hidden.revision))
        .await
        .unwrap();
    assert!(removed.sessions.is_empty());
    assert_eq!(
        removed.removed_sessions,
        vec!["agent-test"],
        "a delta must replay only changes committed after the caller's cursor"
    );

    // Deleting a row that does not exist logs nothing (the SQL delete
    // trigger fires per removed row).
    store.delete_session("agent-test").await.unwrap();
    assert_eq!(
        store
            .inventory_sources(Some(removed.revision))
            .await
            .unwrap()
            .revision,
        removed.revision
    );

    // A cursor ahead of the log cannot be honoured: full resync, not silence.
    let ahead = store
        .inventory_sources(Some(removed.revision + 1))
        .await
        .unwrap();
    assert!(ahead.full);
    assert_eq!(ahead.revision, removed.revision);
    assert!(ahead.sessions.is_empty());
}

#[tokio::test]
async fn inventory_never_needs_message_previews_or_project_bodies() {
    let store = mem().await;
    store
        .create_session(&session("agent-preview", None, 1))
        .await
        .unwrap();
    // A large message body is not part of the inventory projection.
    let body = "x".repeat(1024 * 1024);
    store
        .append_message("agent-preview", &Message::user("message", &body))
        .await
        .unwrap();

    store.create_todo(&todo("todo", 1)).await.unwrap();
    let mut record = run("run", "todo", 2);
    record.plan_md = Some(body.clone());
    record.output_md = Some(body);
    record.input_snapshot = Some("not json".into());
    record.trace_manifest = Some("not json".into());
    store.create_todo_run(&record).await.unwrap();

    let full = store.inventory_sources(None).await.unwrap();
    assert_eq!(full.sessions.len(), 1);
    assert_eq!(full.project_runs.len(), 1);
    assert_eq!(full.project_runs[0].id, "run");
    assert_eq!(full.project_runs[0].todo_id, "todo");
    assert_eq!(full.project_runs[0].created_at, 2);
    assert_eq!(full.project_runs[0].status, ProjectTodoRunStatus::Running);

    // Body-only updates are not tracked by the change log.
    store
        .patch_todo_run(
            "run",
            &ProjectTodoRunPatch {
                output_md: Some("changed".into()),
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
    let idle = store.inventory_sources(Some(full.revision)).await.unwrap();
    assert_eq!(idle.revision, full.revision);
    assert!(idle.project_runs.is_empty());
    assert!(idle.removed_runs.is_empty());

    // The status transition is tracked.
    store
        .patch_todo_run(
            "run",
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                ..Default::default()
            },
            0,
        )
        .await
        .unwrap();
    let changed = store.inventory_sources(Some(idle.revision)).await.unwrap();
    assert!(changed.revision > idle.revision);
    assert_eq!(changed.project_runs.len(), 1);
    assert_eq!(changed.project_runs[0].status, ProjectTodoRunStatus::Done);
    assert_eq!(changed.removed_runs, vec!["run"]);

    // Deleting the owning todo cascades the run row into the change log.
    assert!(store.delete_todo("todo").await.unwrap());
    let removed = store
        .inventory_sources(Some(changed.revision))
        .await
        .unwrap();
    assert!(removed.project_runs.is_empty());
    assert_eq!(removed.removed_runs, vec!["run"]);
}

#[tokio::test]
async fn subagent_membership_changes_invalidate_the_session_lane() {
    let store = mem().await;
    for id in ["agent-parent", "agent-child"] {
        store.create_session(&session(id, None, 1)).await.unwrap();
    }
    let before = store.inventory_sources(None).await.unwrap();
    assert_eq!(before.sessions.len(), 2);

    // A child with an active subagent task leaves the top-level lane; the
    // task insert is staged against the child session id.
    store
        .create_subagent_task(&subagent("agent-child"))
        .await
        .unwrap();
    let child = store
        .inventory_sources(Some(before.revision))
        .await
        .unwrap();
    assert!(child.sessions.is_empty());
    assert_eq!(child.removed_sessions, vec!["agent-child"]);
    assert!(child.revision > before.revision);

    // Removing the membership row restores the child. The Store contract has
    // no task-delete entry point, so the public equivalent is the
    // parent-session cascade (which deletes the task and stages both ids).
    store.delete_session("agent-parent").await.unwrap();
    let restored = store.inventory_sources(Some(child.revision)).await.unwrap();
    assert_eq!(
        restored
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-child"]
    );
    assert!(restored
        .removed_sessions
        .iter()
        .any(|id| id == "agent-child"));
}
