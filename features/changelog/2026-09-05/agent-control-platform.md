Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# Agent 平台：控制面与节点执行闭环

日期：2026-09-05

## 需求与实现

将 daemon 的集中执行拆成独立 `opencoder-server` 控制面和 `opencoder-agent` 节点执行，保留 CLI/TUI。新增 `opencoder-control`、`opencoder-worker`，复用已有 agent/session/team/DAG/TODO/project 引擎与 Web 资源管理页面。

- 节点主动建立 Bearer 鉴权的 WebSocket；稳定节点/维护 agent 身份、重连、心跳、即时负载及索引同步。
- Server 的执行索引严格为 `id / created_at / kind / node_id / status`。输入、消息、事件、结果、团队 topic、项目运行、DAG 产物均在 Node，明细按 ID 转发查询。
- 按真实活跃 agent loops / 可用 CPU 调度，包含子 agent；并发派发使用预留，指定节点及项目 Plan → Act 保持节点归属。
- Node 接受前 fsync 执行记录与资源快照；同 ID 幂等、不同输入冲突、模糊网络错误不改派。断线继续运行，重启未完成任务变 interrupted，仅显式恢复。持久化失败上报不可调度。
- 普通 team/workflow 全部在同一 Node；新建 System 执行和跨节点团队调用已关闭。维护由用户明确触发。
- NFS 只共享 agent 资源，Node 校验只读 NFS 并固定当前版本的实际文件；定义发布或删除不改变已接受的执行。TODO 模板绑定环境在派发时校验并固定。
- 大脑能力绑定 agent/team/DAG/TODO，支持预览及按稳定请求 ID 直接派发。
- 项目结构保存在 Server，运行计划/执行在 Node；CLI/旧 daemon 历史保留，平台使用独立新库，不读取旧 MySQL 项目数据。
- Web 新增节点负载/维护、统一执行、团队成员职责、能力绑定/调度、节点明细和分页产物下载；保留会话、项目、DAG/TODO 等原有入口并适配转发。
- 创建入口统一使用可重试 ID；真实浏览器发现 `randomUUID` 缺失导致提交中断，改用安全随机字节生成 ID。旧 Chat 不再在会话错误后悄悄创建新会话。

资源目录采用任务局部作用域；runner 入口和原 agent 测试拆分为小模块。新工具链对部分 HTTP 错误类型的 lint 增加局部技术说明，保持既有 Axum Response 边界；修正旧团队 DOM 测试在异步内容到达前读取的时序问题。

部署与边界见 [Agent 调度平台](../../../docs/agent-platform.md)。内核 NFS 挂载和真实 runc rootfs 未在本次环境验收；已有 in-process Python VM 的强制中断限制保留并明确记录。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| CPU 比例、预留、固定节点、失联排除 | `cpu_weight_reservations_pinning_and_offline` | `crates/core/src/fleet/scheduling.rs` |
| 真实 loop 计数、嵌套去重、异常释放 | `nested_drains_and_unwind_release_their_registration` | `crates/session/src/loop_registry/mod.rs` |
| 容器小数 CPU | `fractional_and_unlimited_cpu` | `crates/node/src/fleet/cpu.rs` |
| 五字段索引、类型和归属不变、定义分离 | `immutable_owner_kind_and_creation_time_with_definition_separation` | `crates/store/src/fleet/records.rs` |
| 并发资源作用域隔离 | `concurrent_scopes_do_not_change_the_global_root` | `crates/core/src/agent/scope.rs` |
| 接受幂等、本地明细、后续会话 | `durable_acceptance_deduplicates_and_details_stay_on_node` | `crates/worker/tests/durable_execution/main.rs` |
| 重启与显式恢复 | `restart_marks_unfinished_work_interrupted_and_requires_explicit_resume` | `crates/worker/tests/durable_execution/main.rs` |
| 真实 WS、断线继续执行、Server 无明细 | `server_routes_by_id_and_disconnect_does_not_stop_accepted_work` | `crates/worker/tests/fleet_channel.rs` |
| 普通团队职责与本地执行 | `team_members_execute_locally_with_role_assignments` | `crates/worker/tests/workloads.rs` |
| DAG 本地产物、检查点恢复 | `dag_artifacts_and_checkpoints_survive_node_restart` | `crates/worker/tests/workloads.rs` |
| TODO 父子执行在同节点 | `todo_parent_and_children_complete_in_one_node` | `crates/worker/tests/workloads.rs` |
| 维护 agent 的真实工具调用 | `maintenance_agent_has_real_local_query_tool` | `crates/worker/tests/workloads.rs` |
| 大脑预览、绑定、派发幂等 | `brain_preview_binding_and_dispatch_are_idempotent` | `crates/worker/tests/platform/main.rs` |
| 项目 Plan → Act 与新草稿节点归属 | `project_plan_act_and_new_draft_stay_on_the_assigned_node` | `crates/worker/tests/platform/workloads.rs` |
| 普通团队保持单节点、禁止新 System 执行 | `ordinary_team_stays_on_one_node_and_system_creation_is_retired` | `crates/worker/tests/platform/workloads.rs` |
| 资源快照不受发布/移除影响 | `pinned_resources_survive_publish_and_resource_removal` | `crates/worker/tests/resource_snapshot.rs` |
| 缺少资源拒绝接受 | `missing_referenced_resources_fail_preflight` | `crates/worker/tests/resource_snapshot.rs` |
| 环境绑定快照与缺失工具拒绝 | `bound_environment_is_pinned_and_missing_tools_reject_dispatch` | `crates/control/src/api/template.rs` |
| UI 重试沿用 ID、维护显式触发、旧浏览器 ID | `fleet execution boundaries` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |
| 真实双节点与打包 SPA 调度、明细、团队、大脑页面 | `browser-acceptance-platform.js` | `scripts/acceptance/platform.js` |

既有 daemon smoke、节点 smoke、远程会话模式切换和其余 workspace 测试一并回归。

## 验证结果

- SPA：`npm test -- --maxWorkers=2 --minWorkers=2 --testTimeout=15000` → **35 套件 / 335 passed / 0 failed**，输出 `/tmp/opencoder-spa-tests.log`。
- SPA 构建与 `scripts/check-spa-drift.sh` → 通过，无静态资源漂移。
- 真实浏览器：`PLATFORM_BIN_DIR=/data00/rust-build/cargo/opencoder-platform/debug node scripts/acceptance/platform.js` → **PASS**，两个真实节点，页面创建 agent 并查询本地消息，API 派发 Python DAG 并读取节点产物；截图与临时日志 `/tmp/opencoder-platform-browser-fvo4PW/`。使用已安装的 Playwright Chromium；系统自带 Chromium 90 不满足现有 Markdown 依赖要求。
- 全量回归：`cargo test --offline --workspace -j 8` → **311 套件 / 4461 passed / 0 failed / 1 ignored（既有）**，含文档测试，退出码 0；完整输出 `/tmp/opencoder-workspace-tests.log`。
- 静态检查：`cargo clippy --offline --workspace --all-targets -j 8 -- -D warnings` → 零警告，输出 `/tmp/opencoder-clippy.log`。
- 构建：`cargo build --offline --workspace -j 8` → 通过，退出码 0，输出 `/tmp/opencoder-build.log`。
- `git diff --check`、新增代码 ≤ 400 行、修改代码 ≤ 800 行和新增逻辑目录文件数检查均通过。Server 正常依赖树不包含 session/worker/team/project/DAG runtime/Web 执行引擎。
