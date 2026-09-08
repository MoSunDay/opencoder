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
        input_snapshot: None,
        trace_manifest: None,
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

#[tokio::test]
async fn plan_claim_blocks_execute_and_finalization_commits_todo_and_run_together() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_todo(&todo("atomic", ProjectTodoStatus::Planned, 1))
        .await
        .unwrap();
    let mut plan = execute_run("prun-plan", "atomic", 99, 2);
    plan.kind = ProjectTodoRunKind::Plan;
    plan.input_snapshot = Some("{\"input\":\"accepted\"}".into());
    assert!(store.claim_todo_running_with_run(&plan, 2).await.unwrap());
    assert_eq!(
        store.get_todo_run(&plan.id).await.unwrap().unwrap().version,
        1
    );
    assert!(!store
        .claim_todo_running_with_run(&execute_run("prun-act", "atomic", 1, 3), 3)
        .await
        .unwrap());
    let patch = ProjectTodoRunPatch {
        status: Some(ProjectTodoRunStatus::Done),
        output_md: Some("new plan".into()),
        finished_at: Some(4),
        ..Default::default()
    };
    let conn = store.conn().await.unwrap();
    conn.execute("CREATE TRIGGER fail_finalization BEFORE UPDATE ON project_todo_runs BEGIN SELECT RAISE(FAIL, 'injected finalization failure'); END", ()).await.unwrap();
    assert!(store.finish_todo_run(&plan.id, &patch, 4).await.is_err());
    assert_eq!(
        store
            .get_todo("atomic")
            .await
            .unwrap()
            .unwrap()
            .plan_md
            .as_deref(),
        Some("# plan")
    );
    assert_eq!(
        store.get_todo_run(&plan.id).await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Running
    );
    conn.execute("DROP TRIGGER fail_finalization", ())
        .await
        .unwrap();
    assert!(store.finish_todo_run(&plan.id, &patch, 4).await.unwrap());
    assert_eq!(
        store
            .get_todo("atomic")
            .await
            .unwrap()
            .unwrap()
            .plan_md
            .as_deref(),
        Some("new plan")
    );
    assert_eq!(
        store.get_todo_run(&plan.id).await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Done
    );
    assert!(!store
        .finish_todo_run(
            &plan.id,
            &ProjectTodoRunPatch {
                status: Some(ProjectTodoRunStatus::Failed),
                ..Default::default()
            },
            5
        )
        .await
        .unwrap());
    assert!(store
        .claim_todo_running_with_run(&execute_run("prun-act", "atomic", 1, 6), 6)
        .await
        .unwrap());
    assert_eq!(
        store
            .get_todo_run("prun-act")
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );
}

#[tokio::test]
async fn medium_fields_respect_total_page_budget_without_losing_history() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_todo(&todo("pages", ProjectTodoStatus::Planned, 1))
        .await
        .unwrap();
    for version in 1..=25 {
        let mut run = execute_run(&format!("prun-{version}"), "pages", version, version);
        run.status = ProjectTodoRunStatus::Done;
        run.plan_md = Some("p".repeat(60000));
        run.output_md = Some("o".repeat(60000));
        run.input_snapshot = Some("i".repeat(60000));
        run.trace_manifest = Some("m".repeat(60000));
        store.create_todo_run(&run).await.unwrap();
    }
    let mut cursor = None;
    let mut versions = Vec::new();
    loop {
        let page = store
            .list_todo_runs_page("pages", cursor, 20)
            .await
            .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 600000);
        assert!(!page.runs.is_empty());
        versions.extend(page.runs.iter().map(|run| run.version));
        cursor = page.next_version;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(versions, (1..=25).rev().collect::<Vec<_>>());
}
