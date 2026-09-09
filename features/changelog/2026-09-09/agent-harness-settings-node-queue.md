Commit: (working-tree, 基于 303b95027b49873a9833393d57b72de68747a9e0)

# Agent 配置、Codex 参数管理与 Node 并发队列

## 行为

- Agent 配置改为顶部四个 tabs：Agent 列表、Agent Harness、Harness 管理、NFS 配置。主表展示每个 Agent 的 prompt/skills/tools/memory 引用、当前版本、NFS 相对目录和内容名称，列表每次读取当前资源内容。
- 内置及自定义 Agent 均可配置 Harness。Codex 二进制、模型、推理强度、sandbox、approval policy 和 env 由 Harness 管理统一保存，修订号递增；值存入现有私有定义库，不进入 NFS 卡片或公开执行详情。
- 任务接受时固定受管 Codex 参数；pending、续聊、恢复和 fork 使用原快照，受管任务拒绝单次 model/env 覆盖。原生 Agent 保留自己的环境配置。未填写的 Codex 选项由节点 Codex 自身配置决定。
- Node 调度配置保存最大顶层并发数及 FIFO/LIFO。优先选择有空余容量的节点；满载节点仍可持久化接受 pending 任务，释放容量后按顺序启动。调整上限不打断已有执行，配置与尚未启动的队列跨 Node 重启保留。
- 空闲会话后续输入同样受容量限制；重复待调度输入返回原 pending，不覆盖已有输入。Project 排队保留运行预约，避免被失联扫描误判；重复尝试回执与运行索引均显示 pending，取消未启动任务不会调用模型。
- 收敛新增作用域对深层 TODO 编排的栈占用：作用域直接返回 Future，工作负载 Future 与队列快照使用堆分配，默认线程栈下完整 Codex 编排通过。

## 接口与兼容

- `GET /api/harnesses`、`PUT /api/harnesses/codex`。
- `PUT /api/nodes/:id/scheduling`：`{"max_runs":4,"queue_order":"fifo"}`，并发范围 1–65535。
- Fleet 协议 5 → 6，Server 与 Node 必须成套升级；旧协议连接明确拒绝。本轮不新增表、数据库迁移或部署环境变量。
- Node 私有 `scheduling.json` 保存上限与顺序，执行 journal 保存配置快照及队列。运行中任务重启后仍需显式恢复；未开始的任务继续排队。发布冻结时 pending 不启动，现有备份流程要求先逐条 interrupt 等待任务并确认收敛，不能用旧二进制直接继续新的队列。
- 本轮验证使用隔离 Server、Node、临时数据库及 Codex 入口，未更新正在运行的服务，未操作生产鉴权数据。

## 测试覆盖

| 功能 | 测试名或入口 | 文件 |
| --- | --- | --- |
| Codex 参数校验、字面 env 与错误值 | `settings_validate_and_preserve_literal_environment` | `crates/core/src/harness/settings.rs` |
| FIFO/LIFO 和并发范围 | `queue_order_and_capacity_are_explicit` | `crates/core/src/fleet/queue.rs` |
| 满载调度、优先空余容量、pin/ready/过期拒绝 | `full_nodes_queue_by_pending_count_without_bypassing_readiness_or_pinning` | `crates/core/src/fleet/scheduling.rs` |
| 受管参数传入真实进程、排队期间配置不漂移、FIFO/LIFO、非法覆盖 | `managed_codex_is_pinned_and_node_obeys_fifo_lifo` | `crates/worker/tests/harness_settings_queue.rs` |
| 动态上限、取消 pending 不启动 | `dynamic_limit_does_not_interrupt_active_work_and_pending_cancel_never_starts` | 同上 |
| Node 重启保留队列和配置 | `pending_queue_and_scheduling_survive_node_restart` | 同上 |
| 空闲续聊排队、幂等和原参数 | `idle_codex_followup_waits_for_capacity_and_keeps_its_settings` | 同上 |
| Project 排队回执、按容量启动、取消零模型调用 | `project_queue_preserves_pending_receipts_and_cancel_before_start` | `crates/worker/tests/queue_project/main.rs` |
| 待调度 Project 不被失联扫描收敛，不重复 Plan | `reserved_project_waits_without_stale_convergence_and_excludes_another_plan` | `crates/project/src/service_tests/lifecycle.rs` |
| Codex Project/Team/DAG/TODO 与混合执行，无原生凭据 | `codex_orchestrators_without_native_credentials` | `crates/worker/tests/harness_matrix.rs` |
| Harness 表单保存、非法 env、读失败禁止保存、内置及自定义选择、节点设置 | 四项 DOM 用例 | `crates/web/spa/src/harness/management.dom.test.jsx` |
| Agent NFS 内容与版本、启动与返回详情 | Agent 配置 DOM 用例 | `crates/web/spa/src/agentsConfig.dom.test.jsx` |
| 顶部 tabs、管理配置、节点设置、三折叠、刷新、续聊、390px、真实 Project Plan → Execute | 确定性二进制与实际 Codex 两种模式 | `scripts/acceptance/harness/codex.js`、`settings.js` |

## 验证结果

- 全量 `cargo test --workspace`：**4938 passed / 0 failed / 5 ignored**；355 个测试目标结果。5 项为已有手动环境用例，本轮没有新增 ignore。原基线 4929 passed，本轮新增 9 项 Rust 用例。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告；`cargo build --workspace`：通过。
- SPA 全量：**477 passed / 57 files**；构建产物已更新。新增 4 项管理配置 DOM 用例。
- 行数与敏感信息检查：新增代码文件 ≤400 行，修改代码文件 ≤800 行；未发现硬编码凭据。
- 本机完整 Rust 输出：`/var/tmp/opencoder-settings-workspace-complete.log`；lint/build：`/var/tmp/opencoder-settings-clippy-final.log`、`/var/tmp/opencoder-settings-build-final.log`；SPA：`/var/tmp/opencoder-settings-spa-full.log`。
- 浏览器确定性进程验收通过，截图与记录：`/tmp/opencoder-wrap-browser-7LCjJL`。实际 Codex 验收通过：`/tmp/opencoder-wrap-browser-hGbNls`。最终构建二进制的真实 Codex 复验亦通过，截图与 Project 验收记录在 `/tmp/opencoder-wrap-browser-kzPVYQ`，日志为 `/var/tmp/opencoder-settings-browser-final.log`。

回归期间定位并修复深层 TODO 调用链的栈溢出；Project 回放旧用例一次出现超时，独立目标及最终全量均通过，未放宽断言或增加超时。新增 Project 队列测试提供固定会话标题，避免标题生成占用用于控制任务顺序的 Mock 门闩。

相关说明：[Agent Harness](../../../features/harness/index.md)、[Agent 平台](../../../features/agent-platform/index.md)、[节点执行逻辑](../../../agents/worker/index.md)。
