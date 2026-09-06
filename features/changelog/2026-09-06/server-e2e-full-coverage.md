# Server 控制面 e2e 全量覆盖（155 用例）

日期：2026-09-06

## 需求与实现

为 `opencoder-server` 控制面（`opencoder_control::build_app` 完整 `/api` 面：Bearer 鉴权 + Web 资产 + WS 节点通道）建立专属 e2e 套件 `crates/control/tests/e2e/`，与 `opencoder-server` + `opencoder-agent` 部署拓扑一致：真实 axum 路由 + 进程内脚本化 WS 节点（`opencoder_node::fleet::run`）+ 真实 HTTP（reqwest）+ MockChatClient。本轮将全部 server 类型资源的子功能补齐：每个 handler 的查询参数校验、错误分支、分页/游标、SSE 增量与错误帧、幂等/冲突/冻结准入、目录解析（dag/team/todos/project target）、资源池版本/回滚/引用卡、NFS 启停、节点状态派生（idle/busy/lost）与 drain 聚合。

- 测试基建（`support/`）：MockNode 表驱动脚本 + 真协议回退（Create 日志幂等、64KiB 产物分片）；本轮新增旋钮：`set_events_more`（more 分页）、`set_events_status`（SSE 错误帧）、`set_snapshot_opts`（busy/ready 派生 + 单调 snapshot sequence）、`set_admission_reply`（freeze/reopen 拒绝路径）、`set_create_reply`/`clear_create_reply`（428 重试）、`set_artifact_raw`（不一致分片元数据 502 矩阵）、`journal_ids`、`freezes_count`、`Harness::req_bytes`（raw body + 任意 header，如 `last-event-id`）。
- 24 个模块、155 用例（原 46 → 155），全部确定性（有界轮询/超时，无盲等）。
- 纯函数式：表驱动断言 + 无隐藏状态机，遵循仓库禁 class 规则；新文件均 <400 行、迭代文件 <800 行。
- 分层定位（rules/03）：本套件属集成层——MockChatClient 驱动、随常规 `cargo test` 运行；「e2e」指部署拓扑保真（真 `build_app` 路由 + 真 WS 节点协议 + 真 HTTP 客户端），真 LLM 端到端仍归 `scripts/e2e-glm.sh`。

## Impact Surface

- 新增：`crates/control/tests/e2e/`（24 模块 + support 基建，共 ~9.6k 行，全部测试代码）。
- 不改任何生产代码；仅测试基建与用例。

## 测试覆盖（按资源族）

| 资源/子功能 | 文件（tests/e2e/） | 用例数 |
| --- | --- | --- |
| 基建旋钮回归 | support_knobs.rs | 8 |
| infra：SPA/静态资产/time/health/ready/404/favicon/SW 头/非 Bearer/单调时间 | infra_static.rs | 9 |
| admin drain：冻结-重开循环、活跃执行聚合、offline_nodes、重开拒绝保持冻结、未确认 freeze | admin_drain.rs、fleet_admin_extra.rs | 8 |
| fleet：节点目录派生（idle/busy/lost）、维护 RPC 透传与准入门、WS 错误 Bearer | fleet_maintenance.rs | 6 |
| executions 提交：node_id 校验/固定、409 不同输入、无可用节点、目录解析（dag/team/todos/project target） | executions_submit.rs | 4 |
| executions 核心：路由归属、命令动词（cancel/interrupt/plan/execute 快照注入）、节点 404 透传 | executions_core.rs | 7 |
| executions 分页：limit 边界、cursor 校验、next_cursor 全页遍历 | executions_paging.rs | 5 |
| executions 流：last-event-id 恢复、增量尾推、more 分页去重、SSE 错误帧、payload/detail-field/messages offset 重组 | executions_streams.rs | 7 |
| executions 产物：控制面 404、零字节、总长/文件名净化、不一致分片 502 矩阵、原始分片脚本 | executions_artifacts.rs | 7 |
| sessions：创建（自定 id/固定/校验/冻结）、降级行、relay 动词矩阵/遍历/2MiB/非 JSON/节点错误透传、准入尾集 | sessions_relay.rs | 12 |
| compat：models/skills 委派与 503、任务创建/续接/取消、dialogs 降级与节点作用域 | compat_nodes.rs、sessions_compat_extra.rs | 9 |
| todo 模板：创建/校验、meta/context/env 绑定与墓碑、版本 fork/删除规则 | todo_templates_extra.rs | 4 |
| todo 环境/工具/派发：env CRUD 合并语义、工具并集、env 固定派发达节点、缺失工具/篡改 spec 400 | todo_workflows.rs | 7 |
| dag：defs 校验表/裸 spec/bare delete、派发自增 id/幂等/前缀冲突/固定/428 重试、列表降级、after 游标、workflow 404 | dag_runs.rs、teams_dag_defs.rs、dag_dispatch_extra.rs | 18 |
| project：三级 CRUD 全字段/级联/重挂、overview 嵌套/活状态合并/detail_error、plan/execute 快照与 404、取消 | project_api.rs、project_crud_extra.rs | 18 |
| brain：能力校验/搜索 k 默认与钳制、target 守卫/重绑、plans 门与规划故障、动态预览、replan、top_k/model、keyed 409/未确认 503、team 目标派发 | brain_api.rs、brain_dispatch_extra.rs | 15 |
| agents：active 指针（停用/空白/幂等/预检）、卡片历史/引用快照、资源池校验矩阵/版本不复用/固定版本取文件/引用删除 409、NFS 启停幂等 | agents_api.rs、agents_resources_extra.rs | 11 |

- 全量回归：`cargo test --workspace` → 全部通过（见下）；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo fmt --all --check` 干净。
- e2e 套件：`cargo test -p opencoder-control --test e2e` → 155 passed / 0 failed（连续 3 次确定性）。

## 部署备注

纯测试增量，无生产行为变化；`opencoder-control` CI 时间增加约 10s。
