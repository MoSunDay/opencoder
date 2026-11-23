use super::*;
use crate::turso_store::schema;
use crate::types::{SessionMeta, TASK_TYPE_AGENT_STEP, TASK_TYPE_PARENT, TASK_TYPE_SUBAGENT};

async fn memory() -> Connection {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    schema::bootstrap(&conn).await.unwrap();
    conn
}

#[allow(clippy::too_many_arguments)]
fn meta(
    id: &str,
    created_at: i64,
    updated_at: i64,
    task_type: &str,
    kind: Option<&str>,
    workdir_hash: Option<&str>,
) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: None,
        workdir_hash: workdir_hash.map(str::to_string),
        created_at,
        updated_at,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: Some(task_type.into()),
        requirement: None,
        kind: kind.map(str::to_string),
    }
}

/// Activity order (desc): s_oper(9_000, operator lane) > s_sub(8_000, subagent
/// child) > s_hot(7_000) > s_mid(5_000, other workdir) > s_step(1_000,
/// agent step) > s_old(900) > s_old2(900, older id) > s_never(300).
async fn seed(conn: &Connection) {
    for (id, created_at, updated_at) in [
        ("s_oper", 9_000, 9_000),
        ("s_sub", 8_000, 8_000),
        ("s_hot", 7_000, 7_000),
        ("s_mid", 1_500, 5_000),
        ("s_step", 1_000, 1_000),
        ("s_old", 900, 900),
        ("s_old2", 900, 900),
        ("s_never", 300, 0),
    ] {
        let (task_type, kind, workdir_hash) = match id {
            "s_oper" => (TASK_TYPE_PARENT, Some("operator"), None),
            "s_sub" => (TASK_TYPE_SUBAGENT, None, None),
            "s_mid" => (TASK_TYPE_PARENT, None, Some("other")),
            "s_step" => (TASK_TYPE_AGENT_STEP, None, None),
            _ => (TASK_TYPE_PARENT, None, None),
        };
        super::super::create(conn, &meta(id, created_at, updated_at, task_type, kind, workdir_hash))
            .await
            .unwrap();
    }
    conn.execute(
        "INSERT INTO subagent_tasks(task_id,parent_session_id,child_session_id,agent,prompt,status,started_at) VALUES('t1','s_hot','s_sub','act','p','done',1)",
        (),
    )
    .await
    .unwrap();
}

async fn page_and_fallback(
    conn: &Connection,
    filter: &SessionFilter,
    include_execution_details: bool,
) -> (Vec<SessionListItem>, Vec<SessionListItem>) {
    let plan = ListPlan::build(filter, include_execution_details).unwrap();
    (
        fetch_page(conn, &plan).await.unwrap(),
        list_via_point_lookups(conn, &plan).await.unwrap(),
    )
}

fn ids(items: &[SessionListItem]) -> Vec<&str> {
    items.iter().map(|s| s.id.as_str()).collect()
}

#[tokio::test]
async fn point_lookups_reproduce_the_single_statement_page() {
    let conn = memory().await;
    seed(&conn).await;
    let mut filter = SessionFilter::default();
    let cases: Vec<SessionFilter> = vec![
        filter.clone(),
        {
            filter.limit = 2;
            filter.clone()
        },
        {
            filter.limit = 3;
            filter.cursor = Some("900|s_old".into());
            filter.clone()
        },
        {
            filter.cursor = Some("300|s_never".into());
            filter.clone()
        },
        SessionFilter {
            search: Some("s_o".into()),
            ..SessionFilter::default()
        },
        SessionFilter {
            search: Some("title-s_mid".into()),
            ..SessionFilter::default()
        },
        SessionFilter {
            workdir_hash: Some("other".into()),
            ..SessionFilter::default()
        },
        SessionFilter {
            kind: Some("operator".into()),
            ..SessionFilter::default()
        },
        SessionFilter {
            include_subagents: true,
            ..SessionFilter::default()
        },
    ];
    for case in &cases {
        for include_execution_details in [false, true] {
            let (page, fallback) = page_and_fallback(&conn, case, include_execution_details).await;
            assert_eq!(
                ids(&page),
                ids(&fallback),
                "case {case:?} exec={include_execution_details}"
            );
        }
    }
    // Sanity: the default page exercises every fence - the operator lane and
    // the subagent child are hidden, equal activity falls back to id DESC.
    let expected = ["s_hot", "s_mid", "s_step", "s_old2", "s_old", "s_never"];
    let (page, _) = page_and_fallback(&conn, &SessionFilter::default(), false).await;
    assert_eq!(ids(&page), expected);
    let (page, _) = page_and_fallback(&conn, &SessionFilter::default(), true).await;
    assert_eq!(ids(&page), expected);
}

#[tokio::test]
async fn point_lookups_start_after_the_cursor_and_stop_at_the_limit() {
    let conn = memory().await;
    seed(&conn).await;
    let plan = ListPlan::build(
        &SessionFilter {
            limit: 2,
            cursor: Some("900|s_old2".into()),
            ..SessionFilter::default()
        },
        false,
    )
    .unwrap();
    // Candidates past s_old2: s_old (900), s_never (300) - and no third round.
    assert_eq!(
        ids(&list_via_point_lookups(&conn, &plan).await.unwrap()),
        ["s_old", "s_never"]
    );
}
