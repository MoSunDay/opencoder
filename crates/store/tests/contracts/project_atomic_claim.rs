use std::sync::Arc;

use opencoder_store::{
    LibsqlStore, ProjectExecutorKind, ProjectStore, ProjectTodoRecord, ProjectTodoRunKind,
    ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};
use tokio::sync::Barrier;

fn todo(id: &str, status: ProjectTodoStatus, now: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
        id: id.into(),
        milestone_id: None,
        title: id.into(),
        draft: "draft".into(),
        plan_md: Some("# plan".into()),
        status,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        executor_ref: None,
        executor_spec: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn execute_run(id: &str, todo_id: &str, version: i64, now: i64) -> ProjectTodoRunRecord {
    ProjectTodoRunRecord {
        id: id.into(),
        todo_id: todo_id.into(),
        kind: ProjectTodoRunKind::Execute,
        version,
        plan_md: Some("# plan".into()),
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: now,
        finished_at: None,
        created_at: now,
    }
}

#[tokio::test]
async fn concurrent_claims_create_exactly_one_run() {
    const CALLERS: usize = 20;
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_todo(&todo("todo", ProjectTodoStatus::Planned, 1))
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(CALLERS));
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..CALLERS {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            let run = execute_run(&format!("run-{n}"), "todo", 1, 10 + n as i64);
            barrier.wait().await;
            let won = store
                .claim_todo_running_with_run(&run, 10 + n as i64)
                .await
                .unwrap();
            (won, run.id)
        });
    }

    let mut winners = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let (won, id) = result.unwrap();
        if won {
            winners.push(id);
        }
    }
    assert_eq!(winners.len(), 1);
    let runs = store.list_todo_runs("todo").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, winners[0]);
    assert_eq!(
        store.get_todo("todo").await.unwrap().unwrap().status,
        ProjectTodoStatus::Running
    );
}

#[tokio::test]
async fn run_insert_failure_rolls_back_claim() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_todo(&todo("target", ProjectTodoStatus::Planned, 1))
        .await
        .unwrap();
    store
        .create_todo(&todo("owner", ProjectTodoStatus::Planned, 2))
        .await
        .unwrap();
    store
        .create_todo_run(&execute_run("collision", "owner", 1, 3))
        .await
        .unwrap();

    let error = store
        .claim_todo_running_with_run(&execute_run("collision", "target", 1, 20), 20)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("insert project todo run"));
    let target = store.get_todo("target").await.unwrap().unwrap();
    assert_eq!(target.status, ProjectTodoStatus::Planned);
    assert_eq!(target.updated_at, 1, "rolled-back claim must not re-stamp");
    assert!(store.list_todo_runs("target").await.unwrap().is_empty());
    assert_eq!(
        store
            .get_todo_run("collision")
            .await
            .unwrap()
            .unwrap()
            .todo_id,
        "owner"
    );
}

#[tokio::test]
async fn existing_running_and_terminal_run_replay_do_not_duplicate() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_todo(&todo("todo", ProjectTodoStatus::Planned, 1))
        .await
        .unwrap();
    let first = execute_run("first", "todo", 1, 10);
    assert!(store.claim_todo_running_with_run(&first, 10).await.unwrap());
    assert!(!store
        .claim_todo_running_with_run(&execute_run("second", "todo", 2, 11), 11)
        .await
        .unwrap());
    assert!(store
        .patch_todo_run(
            "first",
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Done),
                finished_at: Some(20),
                ..Default::default()
            },
            20,
        )
        .await
        .unwrap());
    assert!(store
        .patch_todo(
            "todo",
            &opencoder_store::ProjectTodoPatch {
                status: Some(ProjectTodoStatus::Done),
                ..Default::default()
            },
            20,
        )
        .await
        .unwrap());

    let error = store
        .claim_todo_running_with_run(&first, 30)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("insert project todo run"));
    let runs = store.list_todo_runs("todo").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "first");
    assert_eq!(runs[0].status, ProjectTodoRunStatus::Done);
    let todo = store.get_todo("todo").await.unwrap().unwrap();
    assert_eq!(todo.status, ProjectTodoStatus::Done);
    assert_eq!(todo.updated_at, 20, "failed replay rolls claim back");
}
