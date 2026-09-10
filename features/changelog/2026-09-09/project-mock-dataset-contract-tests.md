Commit: 708220d7e1c553838af70d0d2df7a4e83d9ce406

# 项目 mock 数据集契约测试：总览投影 / 全执行器序列化 / runs 分页

## 背景与行为

纯测试迭代，不改任何生产行为。此前 `/api/project/*` 的用例各自建小数据，七个语义面从未在同一数据集下被钉住：archived goal 的总览投影、四种 executor kind 的同集序列化、runs 游标分页、backlog 多条排序、悬空引用的投影行为、plan 失败路径、活 run 的 HTTP 取消。

- 新增 `tests/support/project_mock.rs`：纯函数 mock 数据集构造器 + seeder。数据集 = 3 goals（含 archived，sort=0 居首）/ 5 milestones（嵌套×3、standalone×1、悬空 goal_id×1）/ 8 todos（draft/planned/running/done/failed 五态 × agent/team/dag/brain 四执行器，backlog 对 created_at 拉开 100ms，孤儿 milestone_id×1）/ 31 runs（t-planned 灌 v1..v23 驱动分页断言，另留 failed/cancelled/活 running 历史）。
- 灌入混合模式：结构行走 HTTP create/PATCH（seeding 同时内联钉住默认状态、默认 agent、executor 三字段归一化等默认值契约）；HTTP 无法表达的行走 `Arc<dyn ProjectStore>` 直灌——悬空引用（API 对不存在父级 404）、需要精确 created_at 的排序敏感行、todo 的 status/plan_md 回写（归 plan/execute 运行时所有，PATCH 刻意不暴露）。
- `Harness` 增加 `pub projects: Arc<dyn ProjectStore>` 字段（内存库同实例双 trait），供 mock 模块精确灌数据。

## 固化的当前语义（characterization）

- 总览投影按 `sort_key` 升序，archived goal 原样包含（嵌套照带）；三区形状 = goals(嵌套) / standalone_milestones / backlog；backlog 按 created_at 排序。
- 悬空引用静默丢弃：指向已删 milestone 的 todo、指向已删 goal 的 milestone 在总览任何区块都不出现，但行本身仍在平铺列表（`GET /milestones`、`GET /todos`）。若评审认为应显式暴露（如归入 backlog），另行立项改语义。
- 活 running run 行的 seeded `started_at` 必须取 seed 时刻：总览读路径带机会式 stale-run 清扫（300s 宽限），陈旧 running 行会被收敛成 failed。
- plan 失败（LLM Error 流）→ run failed 且 `output_md` 保留错误链文本，todo 行完全不动（留 draft、plan_md null），可立即重试；`finish_todo_run` 事务里只有 DONE 的 plan run 才回写 planned+plan_md。
- 活 execute 取消（hang 脚本 + HTTP cancel）→ run cancelled、todo Running→Planned（plan 保留）、二次 cancel 返回 `{"cancelled":false}`（幂等）。
- runs 分页：默认 20 条 version 降序、`next_version` 指向下一页游标（取尽为 null）、`before_version<=0` → 400；`ProjectRunText` 对小文本以裸字符串内联序列化。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 总览投影：archived 原样、三区形状、backlog created_at 排序、悬空引用静默丢弃（平铺列表仍在） | `overview_full_projection_contract` | `crates/web/tests/web_project_mock_dataset.rs` |
| 四种 executor kind 的 `executor_kind/ref/spec` 序列化 round-trip（spec trim 归一化、milestone 过滤） | `todos_list_serializes_all_executor_kinds` | `crates/web/tests/web_project_mock_dataset.rs` |
| runs 游标分页：默认 20 + `next_version`、`before_version` 翻页取尽 23 条、非法游标 400、页内 run 摘要字段 | `runs_pagination_cursor_contract` | `crates/web/tests/web_project_mock_dataset.rs` |
| plan 失败路径：Error 流 → run failed + 错误链留痕、todo 留 draft、可重试恢复 | `plan_failure_keeps_todo_draft` | `crates/web/tests/web_project_runs.rs` |
| 活 run HTTP 取消：`{"cancelled":true}`、run cancelled、todo 回 planned、二次取消 false | `cancel_live_execute_over_http_reverts_todo` | `crates/web/tests/web_project_runs.rs` |
| mock 数据集构造（混合灌入 + 默认值契约断言） | `seed` / `Dataset` | `crates/web/tests/support/project_mock.rs` |

## 回归证据

- 全量回归：`cargo test --workspace -- --test-threads=1` → **4965 passed / 0 failed / 5 ignored**（迭代基线约 4951 passed / 5 ignored；本轮新增 5 项 web 契约测试，余额为同工作树并行迭代新增的 worker/SPA 用例），见 `/tmp/opencoder-mock-dataset-workspace-tests.log`。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`cargo fmt --check -p opencoder-web` → clean。
- 行数 gate：`project_mock.rs` 361 / `web_project_mock_dataset.rs` 266 / `web_project_runs.rs` 329（167→329，800 限内）/ `project_app.rs` 156，均 ≤400。

## 取舍

- control 层不新增用例：其 handler 经 `#[path]` 源码级复用 web、总览复用同一 store 纯函数，且已有 20 个 e2e 用例；在 web 层固化语义性价比最高。
- 全部纯函数、无 class；同毫秒 created_at 的同里程碑 todo 顺序不做断言（`ORDER BY created_at` 无次级键，顺序本就未定义）。

## 相关文档

[项目逻辑](../../../agents/project/index.md)、[Web](../../../agents/web/index.md)、[存储](../../../agents/store/index.md)。
