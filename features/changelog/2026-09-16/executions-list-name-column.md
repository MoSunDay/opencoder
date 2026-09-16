Commit: a78c378f395e3b7ab8b2d33bd6b26e64f40a2d1f

# 全部执行列表新增「名称」列（派发时快照名）

## 背景

「全部执行」页表格只有 ID / 类型 / 创建时间 / 所属节点 / 状态，用户只能靠五字段执行索引（协议 LOCKED，明确注释 "Do not add detail fields"）辨认一行执行。名称数据其实早已持久化在 `execution_assignments.assignment`（派发时冻结的 `request.target` 与 `definition` 快照），且 `compat/workflows.rs::dag_view` 已有「把派发时 `definition.spec.name` 提升为行顶层 `name`」的先例，本次把该契约对齐到通用执行列表。

## 变更

- `crates/store/src/fleet/handoff/receipts.rs`：新增 `FleetStore::execution_names(&[String])`，单条 SQL 用 `json_extract` 从 `execution_assignments` 批量取 `$.request.target`、`$.definition.name`、`$.definition.spec.name`（无 N+1）；新增 `ExecutionNames` 结构（handoff 模块导出）。不改 `execution_index` 表结构、不改协议 v10。
- `crates/control/src/api/executions/paging.rs`：`list` 在响应 JSON 层给每行提升顶层 `name`（不动 `ExecutionIndex` 结构体）：
  - `team`：优先 `definition.name`，回退 `request.target`
  - `dag`：优先 `definition.spec.name`，再 `definition.name`（内联 spec 形态），回退 `request.target`
  - `agent` / `operator` / `maintenance` / `todos` / `project`：`request.target`
  - `brain` / `system` / 无 assignment 的历史行：不设 `name`（SPA 渲染 `-`）
- `crates/web/spa/src/fleet/executions.jsx`：「类型」列后插入「名称」列（`dataIndex:'name'`，空值渲染 `-`）。
- `crates/web/spa/src/fleet/detail.jsx`：执行详情抽屉标题从裸 id 改为「名称 (id)」，缺名称保持裸 id。

## 影响面与兼容性

- 仅响应字段新增（行顶层 `name`），不删除、不改名既有字段；旧前端可忽略。
- 名称一律读派发时快照：定义后续改名/删除不影响历史行；无 assignment 的 pending/历史行显示 `-`，不阻塞列表。
- 执行索引五字段 DTO 与协议版本不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| store 批量名称查询：多快照形状、缺 id 跳过、空列表短路、prepare 写入可读 | `execution_names_batches_shapes_and_skips_missing_ids` / `execution_names_with_empty_ids_short_circuits` / `execution_names_reads_assignments_written_by_prepare` | `crates/store/src/fleet/handoff/receipts.rs` |
| kind→名称分支：快照优先、target 回退、todos 不吃定义名、brain/system 无名 | `team_and_dag_prefer_the_definition_snapshot_then_fall_back_to_target` / `target_named_kinds_ignore_the_definition_snapshot` / `brain_system_and_incomplete_snapshots_stay_unnamed` | `crates/control/src/api/executions/paging.rs` |
| e2e：提交 agent/team/dag 后列表行 name 正确；删定义后快照名仍在；无 assignment 行无 name | `execution_list_lifts_dispatch_time_names` | `crates/control/tests/e2e/executions_paging.rs` |
| SPA 名称列渲染与 `-` 回退 | `renders the dispatch-time name column with a dash fallback` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |
| 名称列 + 抽屉标题「名称 (id)」（真实 ExecutionDetail） | `renders the name column with a dash fallback and titles the drawer by name` | `crates/web/spa/src/team.dom.test.jsx` |

定向回归：

- `cargo test -p opencoder-store --lib fleet::handoff`
- `cargo test -p opencoder-control --lib executions::paging`
- `cargo test -p opencoder-control --test e2e execution_list_lifts_dispatch_time_names`
- `npx vitest run src/fleet/fleet.dom.test.jsx src/team.dom.test.jsx`（`crates/web/spa`）

产物：`crates/web/spa/dist` 已重建，`scripts/check-spa-drift.sh` 无 drift。
