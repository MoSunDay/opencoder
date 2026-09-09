//! `/api/project/*` 全矩阵特征化（characterization）数据集测试：`seed`
//! 出一个混合数据集（HTTP 建的常规行 + store 直写的悬空/时间敏感行），
//! 驱动三个契约断言——总览树投影、todo 列表对四种 executor 的序列化、
//! run 分页游标。断言的是「当前语义」：悬空引用在总览投影里被静默丢弃
//! （行本身仍在平铺列表里）这类行为一旦改变，这里会先红。

mod support;

use axum::http::StatusCode;
use serde_json::Value;
use support::project_app::{call, harness};
use support::project_mock::{seed, Dataset, DANGLING_MILESTONE};

/// 从 JSON 数组字段收集 id（借用原字符串）。
fn ids_of(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect()
}

/// 按 id 从 `GET /todos` 的 body 里取一行（clone 便于链式断言）。
fn find(list: &Value, id: &str) -> Value {
    list["todos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .cloned()
        .unwrap()
}

/// 一页 runs 的 version 列表。
fn page_versions(page: &Value) -> Vec<i64> {
    page["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["version"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn overview_full_projection_contract() {
    let h = harness().await;
    let ds: Dataset = seed(&h.app, &h.projects).await;
    let (status, tree) = call(&h.app, "GET", "/api/project/overview", None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");

    let goals = tree["goals"].as_array().unwrap();
    assert_eq!(goals.len(), 3, "{tree}");
    // sort_key 升序：归档目标(0) → 目标一(1) → 目标二(2)。
    assert_eq!(goals[0]["id"], ds.g0);
    assert_eq!(goals[0]["status"], "archived", "归档状态原样带入投影");
    assert_eq!(goals[0]["milestones"].as_array().unwrap().len(), 0);

    // g1：两个里程碑，m1a planned / m1b in_progress。
    assert_eq!(goals[1]["id"], ds.g1);
    let g1_ms = goals[1]["milestones"].as_array().unwrap();
    assert_eq!(g1_ms.len(), 2);
    assert_eq!(g1_ms[0]["id"], ds.m1a);
    assert_eq!(g1_ms[0]["status"], "planned");
    assert_eq!(g1_ms[1]["id"], ds.m1b);
    assert_eq!(g1_ms[1]["status"], "in_progress");
    // 同一毫秒创建的两条 todo：库内顺序未定义，只做集合断言。
    let m1a_todos = ids_of(&g1_ms[0]["todos"]);
    assert_eq!(m1a_todos.len(), 2, "{m1a_todos:?}");
    assert!(m1a_todos.contains(&ds.t_draft.as_str()), "{m1a_todos:?}");
    assert!(m1a_todos.contains(&ds.t_planned.as_str()), "{m1a_todos:?}");
    assert_eq!(ids_of(&g1_ms[1]["todos"]), [ds.t_running.as_str()]);

    // g2：done 里程碑挂完成任务。
    assert_eq!(goals[2]["id"], ds.g2);
    let g2_ms = goals[2]["milestones"].as_array().unwrap();
    assert_eq!(g2_ms.len(), 1);
    assert_eq!(g2_ms[0]["id"], ds.m2);
    assert_eq!(g2_ms[0]["status"], "done");
    assert_eq!(ids_of(&g2_ms[0]["todos"]), [ds.t_done.as_str()]);

    // 独立专项不带 goal；brain todo 挂在它下面。
    let standalone = tree["standalone_milestones"].as_array().unwrap();
    assert_eq!(standalone.len(), 1);
    assert_eq!(standalone[0]["id"], ds.ms);
    assert_eq!(ids_of(&standalone[0]["todos"]), [ds.t_failed.as_str()]);

    // backlog：store 直写的精确时间戳 → 顺序确定；全部无里程碑。
    assert_eq!(
        ids_of(&tree["backlog"]),
        [ds.b_early.as_str(), ds.b_late.as_str()]
    );
    assert!(tree["backlog"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["milestone_id"].is_null()));

    // 悬空引用在投影里静默消失（当前语义的特征化）：整体序列化后不含
    // 这两个悬空 id（id 是唯一串，串匹配即精确）。
    let flat = tree.to_string();
    assert!(
        !flat.contains(DANGLING_MILESTONE),
        "dangling milestone leaked: {flat}"
    );
    assert!(!flat.contains(&ds.t_orphan), "orphan todo leaked: {flat}");

    // 但平铺列表仍承载它们：行还在，只是不进投影。
    let (_, milestones) = call(&h.app, "GET", "/api/project/milestones", None).await;
    assert!(ids_of(&milestones["milestones"]).contains(&DANGLING_MILESTONE));
    let (_, todos) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert!(ids_of(&todos["todos"]).contains(&ds.t_orphan.as_str()));
}

#[tokio::test]
async fn todos_list_serializes_all_executor_kinds() {
    let h = harness().await;
    let ds = seed(&h.app, &h.projects).await;
    let (status, list) = call(&h.app, "GET", "/api/project/todos", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["todos"].as_array().unwrap().len(), 8, "{list}");

    // 每行 round-trip 基本列。
    for row in list["todos"].as_array().unwrap() {
        assert!(
            row["title"].as_str().is_some_and(|t| !t.is_empty()),
            "{row}"
        );
        assert!(
            row["draft"].as_str().is_some_and(|t| !t.is_empty()),
            "{row}"
        );
        assert!(row["status"].as_str().is_some(), "{row}");
        assert!(row["created_at"].as_i64().unwrap() > 0, "{row}");
        assert!(row["updated_at"].as_i64().unwrap() > 0, "{row}");
    }

    // team：ref 保留、spec 为空、store 回写的 plan_md 可见。
    let t = find(&list, &ds.t_planned);
    assert_eq!(t["executor_kind"], "team", "{t}");
    assert_eq!(t["executor_ref"], "fleet-x", "{t}");
    assert!(t["executor_spec"].is_null(), "{t}");
    assert_eq!(t["status"], "planned");
    assert_eq!(t["plan_md"], "# 团队方案");
    assert_eq!(t["agent"], "act");

    // dag：内联 spec 按裁剪后的精确串往返；运行中带活跃会话。
    let t = find(&list, &ds.t_running);
    assert_eq!(t["executor_kind"], "dag", "{t}");
    assert!(t["executor_ref"].is_null(), "{t}");
    assert_eq!(t["executor_spec"], ds.dag_spec, "{t}");
    assert_eq!(t["status"], "running");
    assert_eq!(t["active_session_id"], "sess-dag-live");

    // brain：ref 即能力 id。
    let t = find(&list, &ds.t_failed);
    assert_eq!(t["executor_kind"], "brain", "{t}");
    assert_eq!(t["executor_ref"], "cap-1", "{t}");

    // agent（默认）：ref/spec 双空。
    let t = find(&list, &ds.t_draft);
    assert_eq!(t["executor_kind"], "agent", "{t}");
    assert!(t["executor_ref"].is_null(), "{t}");
    assert!(t["executor_spec"].is_null(), "{t}");

    // milestone_id 过滤：只返回该里程碑下的两条（顺序同上不敏感）。
    let (status, filtered) = call(
        &h.app,
        "GET",
        &format!("/api/project/todos?milestone_id={}", ds.m1a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{filtered}");
    let ids = ids_of(&filtered["todos"]);
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.contains(&ds.t_draft.as_str()), "{ids:?}");
    assert!(ids.contains(&ds.t_planned.as_str()), "{ids:?}");
}

#[tokio::test]
async fn runs_pagination_cursor_contract() {
    let h = harness().await;
    let ds = seed(&h.app, &h.projects).await;
    let base_uri = format!("/api/project/todos/{}/runs", ds.t_planned);

    // 第一页：20 条（上限），23..4 降序，游标指向页尾 version。
    let (status, page) = call(&h.app, "GET", &base_uri, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let versions = page_versions(&page);
    assert_eq!(versions.len(), 20, "{versions:?}");
    assert_eq!(versions[0], 23, "newest first");
    assert_eq!(versions[19], 4);
    assert!(
        versions.windows(2).all(|w| w[0] > w[1]),
        "strictly descending: {versions:?}"
    );
    assert_eq!(page["next_version"], 4, "{page}");

    // 页内序列化：ProjectRunText untagged——小文本就是裸字符串。
    assert_eq!(page["runs"][0]["kind"], "plan", "{page}");
    assert_eq!(page["runs"][0]["agent"], "plan");
    assert_eq!(page["runs"][0]["executor_kind"], "team");
    assert_eq!(page["runs"][0]["output_md"], "# 团队方案");
    assert!(page["runs"][0]["plan_md"].is_null(), "{page}");
    assert_eq!(page["runs"][1]["kind"], "execute");
    assert_eq!(page["runs"][1]["agent"], "team:fleet-x");
    assert_eq!(page["runs"][1]["plan_md"], "# 团队方案");

    // 显式游标：取 version<4 的尾巴，耗尽后 next_version 为 null。
    let (status, tail) = call(&h.app, "GET", &format!("{base_uri}?before_version=4"), None).await;
    assert_eq!(status, StatusCode::OK, "{tail}");
    assert_eq!(page_versions(&tail), [3, 2, 1]);
    assert!(tail["next_version"].is_null(), "{tail}");

    // 游标走查：跟随 next_version 直到耗尽，23 条不多不少不重。
    let mut collected = versions;
    let mut cursor = page["next_version"].as_i64();
    while let Some(next) = cursor {
        let (_, next_page) = call(
            &h.app,
            "GET",
            &format!("{base_uri}?before_version={next}"),
            None,
        )
        .await;
        collected.extend(page_versions(&next_page));
        cursor = next_page["next_version"].as_i64();
    }
    assert_eq!(collected.len(), 23, "{collected:?}");
    let mut unique = collected.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique, (1..=23).collect::<Vec<_>>(), "{collected:?}");

    // 非法游标（<=0）→ 400 + 共享错误体。
    for bad in ["0", "-5"] {
        let (status, v) = call(
            &h.app,
            "GET",
            &format!("{base_uri}?before_version={bad}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
        assert_eq!(v["ok"], false, "{bad}: {v}");
    }

    // 另一个 todo 的 run 历史：最新在前，失败留痕齐全。
    let (status, failed) = call(
        &h.app,
        "GET",
        &format!("/api/project/todos/{}/runs", ds.t_failed),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    let runs = failed["runs"].as_array().unwrap();
    assert_eq!(page_versions(&failed), [2, 1]);
    assert_eq!(runs[0]["kind"], "execute", "{failed}");
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["executor_kind"], "brain");
    assert_eq!(runs[0]["capability_id"], "cap-1");
    assert_eq!(runs[0]["output_md"], "执行器崩溃");
    assert_eq!(runs[1]["kind"], "plan");
    assert_eq!(runs[1]["status"], "done");
}
