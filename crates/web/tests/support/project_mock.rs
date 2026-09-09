//! `/api/project/*` 特征化测试的全量 mock 数据集构造器（纯函数 + 一个
//! 异步 seeder），供 `tests/web_project_mock_dataset.rs` 使用。
//!
//! 刻意采用「HTTP 创建 + store 直写」混合模式：
//! - HTTP 能表达的行一律走 HTTP：seeding 的同时内联钉住 create/PATCH 的
//!   默认值契约（默认状态、默认 agent、executor 三字段归一化）。
//! - HTTP 无法表达的行走 `Arc<dyn ProjectStore>` 直写：悬空引用（API 对
//!   不存在的父级直接 404）、需要精确 created_at 的排序敏感行（backlog
//!   对）、以及 todo 的 status/plan_md 回写——它们归 plan/execute 运行时
//!   所有，PATCH 端点刻意不暴露（见 `api_project_todos.rs` 模块注释）。
//!
//! 高密度构造区使用 `#[rustfmt::skip]` 保持紧凑（与 ctl 测试同款做法）。

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use opencoder_core::message::now_ms;
use opencoder_store::{
    ProjectExecutorKind, ProjectMilestoneRecord, ProjectMilestoneStatus, ProjectStore,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};
use serde_json::{json, Value};

use super::project_app::call;

/// goal_id 指向不存在 goal 的悬空里程碑（投影应静默丢弃，平铺列表仍在）。
pub const DANGLING_MILESTONE: &str = "pm-dangling-goal";
/// 指向不存在里程碑的孤儿 todo id。
pub const ORPHAN_TODO: &str = "pt-orphan-milestone";
/// 最小合法内联 DagSpec（通过 `opencode_dag::validate`），作 executor_spec
/// 的往返探针。
pub const DAG_SPEC: &str =
    r#"{"name":"mini","steps":[{"name":"only","kind":{"type":"agent","prompt":"干活"}}]}"#;

/// `seed` 造出的每一行的句柄（HTTP 生成的 id 由服务端 ULID 分配）。
#[rustfmt::skip]
pub struct Dataset {
    /// goals；g0 已归档且 sort=0（总览第一位）。
    pub g0: String, pub g1: String, pub g2: String,
    /// milestones；ms 为无 goal 的独立专项。
    pub m1a: String, pub m1b: String, pub m2: String, pub ms: String,
    /// 覆盖 draft/planned/running/done/failed 五态的五个 todo。
    pub t_draft: String, pub t_planned: String, pub t_running: String,
    pub t_done: String, pub t_failed: String,
    /// store 直写的 backlog 对（created_at 拉开 100ms，排序确定）+ 孤儿 todo。
    pub b_early: String, pub b_late: String, pub t_orphan: String,
    pub dag_spec: &'static str,
}

/// 断言 200 并返回 body（panic 时带上 body）。
async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = call(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{method} {uri}: {v}");
    v
}

/// POST + 取服务端分配的 id。
async fn post_id(app: &Router, uri: &str, body: Value) -> (String, Value) {
    let v = ok(app, "POST", uri, Some(body)).await;
    let id = v["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id: {v}"))
        .to_string();
    (id, v)
}

/// PATCH + 断言 `{"ok":true}`。
async fn patch_ok(app: &Router, uri: &str, body: Value) {
    let v = ok(app, "PATCH", uri, Some(body)).await;
    assert_eq!(v, json!({ "ok": true }), "{uri}: {v}");
}

/// 一条 mock run 行的纯描述（`seed_run` 落库）。
#[rustfmt::skip]
struct RunRow {
    id: String, todo: String, version: i64,
    kind: ProjectTodoRunKind, status: ProjectTodoRunStatus,
    agent: String, executor: ProjectExecutorKind,
    plan_md: Option<String>, output_md: Option<String>,
    capability_id: Option<String>, session_id: Option<String>,
    started_at: i64, finished_at: Option<i64>,
}

/// 终态 run 的公共底座：agent 执行器、无 spec/能力/会话，started/finished
/// 相差 50ms（`..` 结构体更新语法覆盖差异字段）。
#[rustfmt::skip]
fn run_row(id: &str, todo: &str, version: i64, kind: ProjectTodoRunKind, agent: &str, at: i64) -> RunRow {
    RunRow {
        id: id.into(), todo: todo.into(), version, kind, status: ProjectTodoRunStatus::Done,
        agent: agent.into(), executor: ProjectExecutorKind::Agent, plan_md: None,
        output_md: None, capability_id: None, session_id: None,
        started_at: at, finished_at: Some(at + 50),
    }
}

/// 直写一条 run；created_at 与 started_at 一致（runs 列表按 version 排序，
/// 不依赖时间戳）。
#[rustfmt::skip]
async fn seed_run(projects: &Arc<dyn ProjectStore>, row: RunRow) {
    projects
        .create_todo_run(&ProjectTodoRunRecord {
            input_snapshot: None, trace_manifest: None,
            id: row.id, todo_id: row.todo, kind: row.kind, version: row.version,
            plan_md: row.plan_md, output_md: row.output_md, agent: row.agent,
            executor_kind: row.executor, capability_id: row.capability_id, plan_id: None,
            output_ref: None, session_id: row.session_id, status: row.status,
            started_at: row.started_at, finished_at: row.finished_at, created_at: row.started_at,
        })
        .await
        .unwrap();
}

/// 直写一条 draft todo；created_at/updated_at 由调用方精确给定——HTTP 的
/// now() 毫秒级碰撞造不出确定的 backlog 排序，也绕不过父级 404 校验。
#[rustfmt::skip]
async fn direct_todo(
    projects: &Arc<dyn ProjectStore>,
    id: &str, milestone_id: Option<String>, title: &str, draft: &str, at: i64,
) {
    projects
        .create_todo(&ProjectTodoRecord {
            id: id.into(), milestone_id, title: title.into(), draft: draft.into(),
            plan_md: None, status: ProjectTodoStatus::Draft, agent: "act".into(),
            executor_kind: ProjectExecutorKind::Agent, executor_ref: None, executor_spec: None,
            active_session_id: None, created_at: at, updated_at: at,
        })
        .await
        .unwrap();
}

/// store 直写回写 todo 状态机痕迹（status/plan_md 不走 HTTP，理由见模块
/// 注释）。
#[rustfmt::skip]
async fn advance(
    projects: &Arc<dyn ProjectStore>,
    id: &str, status: ProjectTodoStatus,
    plan_md: Option<&str>, session: Option<&str>,
) {
    let applied = projects
        .patch_todo(
            id,
            &ProjectTodoPatch {
                status: Some(status),
                plan_md: plan_md.map(|p| Some(p.to_string())),
                active_session_id: session.map(|s| Some(s.to_string())),
                ..Default::default()
            },
            now_ms(),
        )
        .await
        .unwrap();
    assert!(applied, "store patch_todo({id}) hit nothing");
}

/// 构造全量矩阵数据集：3 goals / 4 milestones / 8 todos / 31 runs。
#[rustfmt::skip]
pub async fn seed(app: &Router, projects: &Arc<dyn ProjectStore>) -> Dataset {
    // ── A. goals：create 默认 active；归档靠 PATCH ───────────────────
    let (g1, v) = post_id(
        app,
        "/api/project/goals",
        json!({ "title": "目标一", "detail_md": "目标一详情", "sort": 1 }),
    )
    .await;
    assert!(g1.starts_with("pg-"), "goal id: {v}");
    assert_eq!(v["status"], "active", "goal create default");
    let (g2, _) = post_id(app, "/api/project/goals", json!({ "title": "目标二", "sort": 2 })).await;
    // sort=0 让归档目标排到总览第一位（list 按 sort_key 升序）。
    let (g0, v) =
        post_id(app, "/api/project/goals", json!({ "title": "归档目标", "sort": 0 })).await;
    assert_eq!(v["status"], "active", "archived goal starts active");
    patch_ok(app, &format!("/api/project/goals/{g0}"), json!({ "status": "archived" })).await;

    // ── milestones：create 默认 planned；无 goal_id 即独立专项 ────────
    let (m1a, v) = post_id(
        app,
        "/api/project/milestones",
        json!({ "goal_id": g1, "title": "里程碑一甲", "sort": 1 }),
    )
    .await;
    assert_eq!(v["status"], "planned", "milestone create default");
    let (m1b, _) = post_id(
        app,
        "/api/project/milestones",
        json!({ "goal_id": g1, "title": "里程碑一乙", "sort": 2 }),
    )
    .await;
    patch_ok(app, &format!("/api/project/milestones/{m1b}"), json!({ "status": "in_progress" }))
        .await;
    let (m2, _) = post_id(
        app,
        "/api/project/milestones",
        json!({ "goal_id": g2, "title": "里程碑二", "sort": 1 }),
    )
    .await;
    patch_ok(app, &format!("/api/project/milestones/{m2}"), json!({ "status": "done" })).await;
    let (ms, v) =
        post_id(app, "/api/project/milestones", json!({ "title": "独立专项", "sort": 1 })).await;
    assert!(v["goal_id"].is_null(), "standalone create keeps goal_id null");

    // ── todos：四种 executor_kind 全覆盖 ─────────────────────────────
    let (t_draft, v) = post_id(
        app,
        "/api/project/todos",
        json!({ "milestone_id": m1a, "title": "草稿任务", "draft": "还没想清楚" }),
    )
    .await;
    assert_eq!(v["status"], "draft", "todo create default");
    assert_eq!(v["agent"], "act", "todo default agent");
    assert_eq!(v["executor_kind"], "agent", "todo default executor");
    let (t_planned, v) = post_id(
        app,
        "/api/project/todos",
        json!({ "milestone_id": m1a, "title": "团队任务", "draft": "交给团队跑",
                "executor_kind": "team", "executor_ref": "fleet-x" }),
    )
    .await;
    assert_eq!(v["executor_kind"], "team", "{v}");
    assert_eq!(v["executor_ref"], "fleet-x", "{v}");
    assert!(v["executor_spec"].is_null(), "team takes no spec: {v}");
    // executor_spec 首尾空白会被裁掉：发送带空白的 spec，断言存的是裁剪后
    // 的精确串（round-trip 探针）。
    let (t_running, v) = post_id(
        app,
        "/api/project/todos",
        json!({ "milestone_id": m1b, "title": "DAG任务", "draft": "走内联DAG",
                "executor_kind": "dag", "executor_spec": format!("  {DAG_SPEC}  ") }),
    )
    .await;
    assert_eq!(v["executor_spec"], DAG_SPEC, "spec stored trimmed");
    let (t_done, _) = post_id(
        app,
        "/api/project/todos",
        json!({ "milestone_id": m2, "title": "完成任务", "draft": "已经搞定" }),
    )
    .await;
    let (t_failed, v) = post_id(
        app,
        "/api/project/todos",
        json!({ "milestone_id": ms, "title": "大脑任务", "draft": "能力路由执行",
                "executor_kind": "brain", "executor_ref": "cap-1" }),
    )
    .await;
    assert_eq!(v["executor_kind"], "brain", "{v}");

    // ── B. store 直写：HTTP 造不出来的行 ────────────────────────────
    let base = now_ms();
    // backlog 对：created_at 拉开 100ms，钉住总览 backlog 排序。
    direct_todo(projects, "pt-backlog-early", None, "积压任务", "最早的积压", base + 100).await;
    direct_todo(projects, "pt-backlog-late", None, "积压任务", "稍晚的积压", base + 200).await;
    // 悬空里程碑：goal_id 指向不存在的 pg-gone（表结构刻意无外键）。
    projects
        .create_milestone(&ProjectMilestoneRecord {
            id: DANGLING_MILESTONE.into(), goal_id: Some("pg-gone".into()),
            title: "悬空专项".into(), detail_md: None,
            status: ProjectMilestoneStatus::Planned, sort: 99,
            created_at: base, updated_at: base,
        })
        .await
        .unwrap();
    // 孤儿 todo：milestone_id 指向不存在的 pm-gone。
    direct_todo(projects, ORPHAN_TODO, Some("pm-gone".into()), "孤儿任务", "指向不存在的里程碑", base)
        .await;
    // 状态机回写：模拟 plan/execute 运行时落下的痕迹。
    advance(projects, &t_planned, ProjectTodoStatus::Planned, Some("# 团队方案"), None).await;
    advance(projects, &t_running, ProjectTodoStatus::Running, None, Some("sess-dag-live")).await;
    advance(projects, &t_done, ProjectTodoStatus::Done, Some("# 完成方案"), None).await;
    advance(projects, &t_failed, ProjectTodoStatus::Failed, Some("# 失败方案"), None).await;

    // ── runs ────────────────────────────────────────────────────────
    // t_planned：v1..=23 交替 plan/execute，全部终态（驱动分页游标走查）。
    for v in 1..=23i64 {
        let plan = v % 2 == 1;
        seed_run(
            projects,
            RunRow {
                executor: ProjectExecutorKind::Team,
                plan_md: (!plan).then(|| "# 团队方案".into()),
                output_md: Some(if plan { "# 团队方案".into() } else { format!("第 {v} 次执行完成") }),
                ..run_row(
                    &format!("prun-mock-tp-{v}"),
                    &t_planned,
                    v,
                    if plan { ProjectTodoRunKind::Plan } else { ProjectTodoRunKind::Execute },
                    if plan { "plan" } else { "team:fleet-x" },
                    base + v,
                )
            },
        )
        .await;
    }
    // t_done：plan → execute 全部成功。
    seed_run(projects, RunRow {
        output_md: Some("# 完成方案".into()),
        ..run_row("prun-mock-td-1", &t_done, 1, ProjectTodoRunKind::Plan, "plan", base + 1)
    }).await;
    seed_run(projects, RunRow {
        plan_md: Some("# 完成方案".into()),
        output_md: Some("全部完成".into()),
        ..run_row("prun-mock-td-2", &t_done, 2, ProjectTodoRunKind::Execute, "act", base + 2)
    }).await;
    // t_failed：plan 成功、execute 崩溃（brain 路由留痕 capability_id）。
    seed_run(projects, RunRow {
        executor: ProjectExecutorKind::Brain,
        output_md: Some("# 失败方案".into()),
        capability_id: Some("cap-1".into()),
        ..run_row("prun-mock-tf-1", &t_failed, 1, ProjectTodoRunKind::Plan, "act", base + 1)
    }).await;
    seed_run(projects, RunRow {
        status: ProjectTodoRunStatus::Failed,
        executor: ProjectExecutorKind::Brain,
        plan_md: Some("# 失败方案".into()),
        output_md: Some("执行器崩溃".into()),
        capability_id: Some("cap-1".into()),
        ..run_row("prun-mock-tf-2", &t_failed, 2, ProjectTodoRunKind::Execute, "brain:cap-1", base + 2)
    }).await;
    // t_running：v1 plan 已完，v2 execute 仍在跑。关键：v2 的 started_at 用
    // seeding 当下的 now_ms() 保持「年轻」——GET /overview 的机会式 stale
    // run 清扫会把超 300s 宽限的无主 running 行收敛成 failed。
    seed_run(projects, RunRow {
        executor: ProjectExecutorKind::Dag,
        output_md: Some("# DAG 方案".into()),
        ..run_row("prun-mock-tr-1", &t_running, 1, ProjectTodoRunKind::Plan, "act", base + 1)
    }).await;
    seed_run(projects, RunRow {
        status: ProjectTodoRunStatus::Running,
        executor: ProjectExecutorKind::Dag,
        plan_md: Some("# DAG 方案".into()),
        session_id: Some("sess-dag-live".into()),
        started_at: now_ms(),
        finished_at: None,
        ..run_row("prun-mock-tr-2", &t_running, 2, ProjectTodoRunKind::Execute, "dag:mini", 0)
    }).await;
    // backlog 对的 run 历史：early 只有 plan；late 多一条被取消的 execute。
    seed_run(projects, RunRow {
        agent: "plan".into(),
        output_md: Some("# 积压方案".into()),
        ..run_row("prun-mock-be-1", "pt-backlog-early", 1, ProjectTodoRunKind::Plan, "plan", base + 1)
    }).await;
    seed_run(projects, RunRow {
        agent: "plan".into(),
        output_md: Some("# 积压方案".into()),
        ..run_row("prun-mock-bl-1", "pt-backlog-late", 1, ProjectTodoRunKind::Plan, "plan", base + 1)
    }).await;
    seed_run(projects, RunRow {
        status: ProjectTodoRunStatus::Cancelled,
        plan_md: Some("# 积压方案".into()),
        ..run_row("prun-mock-bl-2", "pt-backlog-late", 2, ProjectTodoRunKind::Execute, "act", base + 2)
    }).await;

    Dataset {
        g0, g1, g2, m1a, m1b, m2, ms,
        t_draft, t_planned, t_running, t_done, t_failed,
        b_early: "pt-backlog-early".into(),
        b_late: "pt-backlog-late".into(),
        t_orphan: ORPHAN_TODO.into(),
        dag_spec: DAG_SPEC,
    }
}
