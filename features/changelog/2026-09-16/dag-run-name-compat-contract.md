Commit: a78c378f395e3b7ab8b2d33bd6b26e64f40a2d1f

# DAG 运行名称在兼容层响应中顶层化

## 背景

「DAG 工作流」页「运行」tab 的表格早已有「名称」列（`dataIndex:'name'`），但在生产 `opencoder-server`（control 平台）形态下恒显示 `-`。根因是两套 DAG run API 契约不一致：本地 daemon（`crates/web/src/api_dag.rs`）返回的 `DagRunView`（`crates/dag/src/protocol.rs`）顶层带 `name`，而 control 兼容层 `GET /api/dag/runs`、`GET /api/dag/runs/:id`（`crates/control/src/api/compat/workflows.rs::dag_view`）只组装 `execution + dag_id + spec + error`，名称藏在 `spec.name` 里，前端 `dataIndex:'name'` 取不到。运行详情头部也只显示 `运行 <id 前 8 位>`。后端链路（`spec.name → dag_defs.name → execution` 派发快照）本身没有问题。

## 变更

- `crates/control/src/api/compat/workflows.rs`：`dag_view()` 把 `definition.spec.name` 提升为行顶层 `name`（spec 缺名称时不设字段）。列表与详情两个入口同时生效，与本地 daemon 的 `DagRunView` 顶层 `name` 契约对齐。不改节点执行索引五字段 DTO（协议 LOCKED），名称一律从派发时的 `definition.spec` 快照读取，不依赖索引。
- `crates/web/spa/src/dag/runsTable.jsx`：名称列 render 兜底为 `name || spec.name || '-'`，兼容旧 server 与节点离线时 inspect 失败的行。
- `crates/web/spa/src/dag/runDetail.jsx`：头部标题优先显示 `current.name || detail.definition.spec.name`，缺失时回退 id 前 8 位。

## 影响面与兼容性

- 仅响应字段新增（行顶层 `name`），不删除、不改名既有字段；旧前端可忽略。
- Fleet「执行」列表（`fleet/executions.jsx`）行数据为五字段执行索引、无 spec 快照，不在本改动范围内；执行索引协议不变。
- `crates/web/src/api_dag.rs` 本地 daemon 契约不变（本就正常）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 兼容层列表行与详情顶层 `name == spec.name` | `dispatch_creates_run_and_ledger_views_route_to_the_node` | `crates/control/tests/e2e/dag_runs.rs` |
| 本地 daemon run 详情/列表顶层 name | `claim_returns_spec_snapshot_and_second_claim_is_204` | `crates/web/tests/dag_api.rs` |
| SPA 名称列：顶层 name、`spec.name` 回退、全缺显示 `-` | `renders the name column with a spec.name fallback` | `crates/web/spa/src/dag/dag.dom.test.jsx` |

定向回归：

- `cargo test -p opencoder-control --test e2e dag_runs`
- `cargo test -p opencoder-web --test dag_api`
- `npx vitest run src/dag/dag.dom.test.jsx`（`crates/web/spa`）
