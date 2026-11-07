//! Atomic claim + run-insert contracts (ported from the SQL contract suite)
//! and the bounded medium-field pagination/text-chunk contracts.
use super::*;

fn execute_run(id: &str, todo_id: &str, version: i64, now: i64) -> ProjectTodoRunRecord {
    let mut rec = run_record(id, todo_id, version, now, ProjectTodoRunKind::Execute);
    rec.plan_md = Some("# plan".into());
    rec.session_id = None;
    rec
}

#[tokio::test]
async fn concurrent_claims_create_exactly_one_run() {
    const CALLERS: usize = 20;
    let p = mem_iface().await;
    p.create_todo(&todo("todo", None, 1)).await.unwrap();
    p.patch_todo("todo", &tpatch(Some(ProjectTodoStatus::Planned)), 1)
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(CALLERS));
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..CALLERS {
        let p = p.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            let run = execute_run(&format!("run-{n}"), "todo", 1, 10 + n as i64);
            barrier.wait().await;
            let won = p
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
    let runs = p.list_todo_runs("todo").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, winners[0]);
    assert_eq!(
        p.get_todo("todo").await.unwrap().unwrap().status,
        ProjectTodoStatus::Running
    );
}

#[tokio::test]
async fn run_insert_failure_rolls_back_claim() {
    let p = mem_iface().await;
    for id in ["target", "owner"] {
        p.create_todo(&todo(id, None, 1)).await.unwrap();
        p.patch_todo(id, &tpatch(Some(ProjectTodoStatus::Planned)), 1)
            .await
            .unwrap();
    }
    p.create_todo_run(&execute_run("collision", "owner", 1, 3))
        .await
        .unwrap();

    // Global run-id uniqueness: the duplicate id bails before commit, so
    // the whole claim (status flip included) rolls back.
    let error = p
        .claim_todo_running_with_run(&execute_run("collision", "target", 1, 20), 20)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("insert project todo run"));
    let target = p.get_todo("target").await.unwrap().unwrap();
    assert_eq!(target.status, ProjectTodoStatus::Planned);
    assert_eq!(target.updated_at, 1, "rolled-back claim must not re-stamp");
    assert!(p.list_todo_runs("target").await.unwrap().is_empty());
    assert_eq!(
        p.get_todo_run("collision").await.unwrap().unwrap().todo_id,
        "owner"
    );
}

#[tokio::test]
async fn existing_running_and_terminal_run_replay_do_not_duplicate() {
    let p = mem_iface().await;
    p.create_todo(&todo("todo", None, 1)).await.unwrap();
    p.patch_todo("todo", &tpatch(Some(ProjectTodoStatus::Planned)), 1)
        .await
        .unwrap();
    let first = execute_run("first", "todo", 1, 10);
    assert!(p.claim_todo_running_with_run(&first, 10).await.unwrap());
    // Second claim while the todo is running loses without inserting.
    assert!(!p
        .claim_todo_running_with_run(&execute_run("second", "todo", 2, 11), 11)
        .await
        .unwrap());
    assert!(p
        .patch_todo_run(
            "first",
            &rpatch(Some(ProjectTodoRunStatus::Done), Some(20)),
            20
        )
        .await
        .unwrap());
    p.patch_todo("todo", &tpatch(Some(ProjectTodoStatus::Done)), 20)
        .await
        .unwrap();

    // Replaying the terminal claim fails on the duplicate id and rolls the
    // claim back: the todo keeps its terminal status/timestamp.
    let error = p.claim_todo_running_with_run(&first, 30).await.unwrap_err();
    assert!(error.to_string().contains("insert project todo run"));
    let runs = p.list_todo_runs("todo").await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "first");
    assert_eq!(runs[0].status, ProjectTodoRunStatus::Done);
    let rec = p.get_todo("todo").await.unwrap().unwrap();
    assert_eq!(rec.status, ProjectTodoStatus::Done);
    assert_eq!(rec.updated_at, 20, "failed replay rolls claim back");
}

#[tokio::test]
async fn plan_claim_blocks_execute_and_versions_sequence() {
    let p = mem_iface().await;
    p.create_todo(&todo("atomic", None, 1)).await.unwrap();
    p.patch_todo("atomic", &tpatch(Some(ProjectTodoStatus::Planned)), 1)
        .await
        .unwrap();
    let mut plan = execute_run("prun-plan", "atomic", 99, 2);
    plan.kind = ProjectTodoRunKind::Plan;
    plan.input_snapshot = Some("{\"input\":\"accepted\"}".into());
    assert!(p.claim_todo_running_with_run(&plan, 2).await.unwrap());
    // The caller-supplied version is ignored: the claim assigns 1.
    assert_eq!(
        p.get_todo_run("prun-plan").await.unwrap().unwrap().version,
        1
    );
    // While the plan run owns the todo, an execute claim loses.
    assert!(!p
        .claim_todo_running_with_run(&execute_run("prun-act", "atomic", 1, 3), 3)
        .await
        .unwrap());

    // Finalization commits the run row and the todo plan together.
    assert!(p
        .finish_todo_run(
            "prun-plan",
            &ProjectTodoRunPatch {
                output_md: Some("new plan".into()),
                ..rpatch(Some(ProjectTodoRunStatus::Done), Some(4))
            },
            4,
        )
        .await
        .unwrap());
    assert_eq!(
        p.get_todo("atomic")
            .await
            .unwrap()
            .unwrap()
            .plan_md
            .as_deref(),
        Some("new plan")
    );
    assert_eq!(
        p.get_todo_run("prun-plan").await.unwrap().unwrap().status,
        ProjectTodoRunStatus::Done
    );
    // Terminal replay is a no-op false.
    assert!(!p
        .finish_todo_run(
            "prun-plan",
            &rpatch(Some(ProjectTodoRunStatus::Failed), None),
            5
        )
        .await
        .unwrap());
    // A claim on a Planned todo with a completed plan wins and sequences.
    assert!(p
        .claim_todo_running_with_run(&execute_run("prun-act", "atomic", 1, 6), 6)
        .await
        .unwrap());
    assert_eq!(
        p.get_todo_run("prun-act").await.unwrap().unwrap().version,
        2
    );
}

#[tokio::test]
async fn medium_fields_respect_total_page_budget_without_losing_history() {
    let p = mem_iface().await;
    p.create_todo(&todo("pages", None, 1)).await.unwrap();
    for version in 1..=25 {
        let mut run = execute_run(&format!("prun-{version}"), "pages", version, version);
        run.status = ProjectTodoRunStatus::Done;
        run.plan_md = Some("p".repeat(60_000));
        run.output_md = Some("o".repeat(60_000));
        run.input_snapshot = Some("i".repeat(60_000));
        run.trace_manifest = Some("m".repeat(60_000));
        p.create_todo_run(&run).await.unwrap();
    }
    let mut cursor = None;
    let mut versions = Vec::new();
    loop {
        let page = p.list_todo_runs_page("pages", cursor, 20).await.unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 600_000);
        assert!(!page.runs.is_empty());
        versions.extend(page.runs.iter().map(|run| run.version));
        cursor = page.next_version;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(versions, (1..=25).rev().collect::<Vec<_>>());
}

/// Bounded medium-field reads: summaries omit >64KiB columns, keyset run
/// pages stay under the payload budget, and the omitted text is fully
/// recoverable through the chunk API (ported from the bounded-pagination
/// contract).
#[tokio::test]
async fn todo_summary_and_run_pages_bound_medium_fields_with_text_chunks() {
    let p = mem_iface().await;
    let mut rec = todo("project-item", None, 1);
    rec.draft = "d".repeat(65_537);
    rec.plan_md = Some("p".repeat(65_536));
    p.create_todo(&rec).await.unwrap();

    // The inspect projection omits the oversized draft but keeps a plan
    // that is exactly at the 64KiB boundary.
    let summary =
        serde_json::to_value(p.get_todo_summary("project-item").await.unwrap().unwrap()).unwrap();
    assert_eq!(summary["draft"]["omitted"], true);
    assert_eq!(summary["draft"]["total_bytes"], 65_537);
    assert_eq!(summary["draft"]["read_via"], "detail_field");
    assert_eq!(summary["draft"]["field"], "project.todo.project-item.draft");
    assert_eq!(summary["plan_md"].as_str().unwrap().len(), 65_536);

    for version in 1..=25 {
        let mut run = execute_run(&format!("run-{version}"), "project-item", version, version);
        run.status = ProjectTodoRunStatus::Done;
        run.finished_at = Some(version);
        run.session_id = Some(format!("agent-run-{version}"));
        run.output_md = (version == 25).then(|| "x".repeat(70_000));
        p.create_todo_run(&run).await.unwrap();
    }
    let runs = p
        .list_todo_runs_page("project-item", None, 20)
        .await
        .unwrap();
    assert_eq!(runs.runs.len(), 20);
    assert_eq!(runs.next_version, Some(6));
    let head = serde_json::to_value(&runs.runs[0]).unwrap();
    assert_eq!(head["id"], "run-25");
    assert_eq!(head["output_md"]["omitted"], true);
    assert_eq!(head["output_md"]["field"], "project.run.run-25.output_md");
    let summary =
        serde_json::to_value(p.get_todo_run_summary("run-25").await.unwrap().unwrap()).unwrap();
    assert_eq!(summary["output_md"]["omitted"], true);

    // The omitted field streams back byte-for-byte through text chunks.
    let mut output = Vec::new();
    let mut offset = 0u64;
    loop {
        let chunk = p
            .project_text_chunk(
                "run",
                "project-item",
                "run-25",
                "output_md",
                offset,
                64 * 1024,
            )
            .await
            .unwrap()
            .unwrap();
        output.extend_from_slice(&chunk.bytes);
        offset += chunk.bytes.len() as u64;
        if offset == chunk.total_bytes {
            break;
        }
    }
    assert_eq!(output, vec![b'x'; 70_000]);
    let last = p
        .list_todo_runs_page("project-item", runs.next_version, 20)
        .await
        .unwrap();
    assert_eq!(last.runs.len(), 5);
    assert!(last.next_version.is_none());
}

#[tokio::test]
async fn text_chunk_validates_kind_owner_field_and_offset() {
    let p = mem_iface().await;
    p.create_todo(&todo("t1", None, 1)).await.unwrap();
    let mut run = run_record("r1", "t1", 1, 1, ProjectTodoRunKind::Execute);
    run.output_md = Some("hello".into());
    p.create_todo_run(&run).await.unwrap();

    // NULL field reads as None instead of an empty chunk.
    assert!(p
        .project_text_chunk("run", "t1", "r1", "plan_md", 0, 1024)
        .await
        .unwrap()
        .is_none());
    // Owner mismatch resolves to no row: None (the run belongs to a
    // different todo; todo fields need the todo itself as the owner).
    assert!(p
        .project_text_chunk("run", "other", "r1", "output_md", 0, 1024)
        .await
        .unwrap()
        .is_none());
    let chunk = p
        .project_text_chunk("todo", "t1", "t1", "draft", 0, 1024)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chunk.bytes, b"draft t1");
    assert!(p
        .project_text_chunk("todo", "t1", "other", "draft", 0, 1024)
        .await
        .unwrap()
        .is_none());
    // offset == total streams an empty tail; only offsets past the end are
    // hard errors.
    let tail = p
        .project_text_chunk("run", "t1", "r1", "output_md", 5, 1024)
        .await
        .unwrap()
        .unwrap();
    assert!(tail.bytes.is_empty());
    assert_eq!(tail.total_bytes, 5);
    assert!(p
        .project_text_chunk("run", "t1", "r1", "output_md", 6, 1024)
        .await
        .is_err());
    assert!(p
        .project_text_chunk("run", "t1", "r1", "nope", 0, 1024)
        .await
        .is_err());
    assert!(p
        .project_text_chunk("other", "t1", "r1", "output_md", 0, 1024)
        .await
        .is_err());
}
