Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# Agent 与 Operator 远端宿主机会话

普通对话统一由 Client 经 Server 转发到所选节点，复用已有后台会话路径，在远端宿主机执行 Server 保存的 Agent 描述、Skill、Tool 与 Harness。普通会话不启动 TUI、不要求 runc；DAG 继续按既有容器约定执行。

## 行为与边界

- 移除 Agent 卡的 `run_mode` 字段、前端执行模式开关与普通 Agent 容器适配代码。
- Server 准备派发只保存冻结请求和回执，远端持久接收后才发布会话执行索引。拒绝不产生占位会话，迟到确认不覆盖更新的节点状态；恢复上报仍校验冻结归属。
- 节点已接收、尚未初始化会话时，读取返回初始化状态和空消息；未知 ID 保持 404，初始化前取消返回实际终止原因，修复首事件前的 `execution not found`。
- 前端重新打开初始化中或运行中的会话会续接事件。首轮内容不变时保留重试 ID，内容改变时创建新 ID；事件流结束后读取已保存的回答。
- 资源快照保留 Tool 文件的执行权限，保证 Server 注册的脚本能在节点运行。没有新增表或环境变量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 两种模式加载 Server 身份、Skill、Tool，流式输出、重试和重启续聊 | `operator_uses_server_identity_skills_tools_and_resumes_on_host`、`agent_uses_server_identity_skills_tools_and_resumes_on_host` | [remote_identity.rs](../../../tests/operator_e2e/remote_identity.rs) |
| 缺失身份拒绝且不留索引 | `missing_identity_is_rejected_without_a_server_execution_record` | [remote_identity.rs](../../../tests/operator_e2e/remote_identity.rs) |
| 接收确认前无会话记录、拒绝无占位 | `conversation_lists_wait_for_remote_durable_acceptance`、`rejected_conversation_does_not_leave_a_phantom_execution` | [acceptance.rs](../../../crates/control/tests/e2e/sessions/acceptance.rs) |
| 迟到确认不回退状态、归属校验、重连恢复与 DAG 原准入行为 | `conversations_publish_only_remote_acceptance_and_replay_does_not_regress_status`、`invalid_receipt_cannot_publish_an_index_or_replace_its_owner`、`reconnect_report_recovers_lost_acceptance_without_a_local_placeholder`、`dag_dispatch_keeps_its_existing_pending_index` | [conversation_acceptance.rs](../../../crates/store/tests/conversation_acceptance.rs) |
| 资源等待取消不生成会话 | `cancellation_before_remote_acceptance_keeps_conversation_lists_empty` | [resource_admission.rs](../../../crates/control/tests/e2e/resource_admission.rs) |
| 两种模式初始化期间读取、事件游标、取消与最终执行 | `agent_is_readable_while_waiting_for_a_host_slot`、`operator_is_readable_while_waiting_for_a_host_slot`、两项 `*_cancel_before_initialization_returns_the_terminal_reason` | [conversations/main.rs](../../../crates/worker/tests/conversations/main.rs) |
| 冻结 Tool 权限且实际执行 | `frozen_tool_keeps_its_executable_permissions_and_runs` | [snapshot/tests.rs](../../../crates/agents/src/snapshot/tests.rs) |
| 前端初始化订阅、恢复游标与提交重试 | `opens an accepted %s before its first event and subscribes for results` 等 4 项 | [initialization.dom.test.jsx](../../../crates/web/spa/src/chat/initialization.dom.test.jsx) |

真实进程验收使用独立 Server、节点、只读资源挂载和实际命令工具，仅模型回复使用确定性测试数据；两种模式均验证首轮、后续输入、工具产物位于节点目录，以及重启后继续同一执行。浏览器链路见 [chat.js](../../../scripts/acceptance/ui/scenarios/chat.js)。

## 验证结果

证据目录：`/tmp/opencoder-remote-sessions-xoq3ia/`，汇总回执 `accepted.json`。

- 全量 Rust：`cargo test --workspace --no-fail-fast -j 4 -- --test-threads=8`，退出码 0；`workspace-checked.log` 的 460 段结果合计 **5749 passed / 0 failed / 8 ignored**。原有依赖浏览器与原生容器的单独验收由下列 UI、DAG 检查覆盖。
- 全量结束后保留并补齐并行任务新增的多线程测试：临时配置为线程局部，Tokio 工作线程也须建立隔离。4 项会话初始化测试全部通过，断言与 20 秒上限不变；见 `initializing-isolated-threads.log`。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -j 4 -- -D warnings`、`cargo build --workspace --bins --examples -j 4` 全部通过；对应 `fmt-accepted.log`、`clippy-accepted.log`、`build-accepted.log`。
- SPA 全量 **1153 passed**，见 `spa-current.log`。全站 UI **17 项通过**，包含 1920/1280/768/390 四种屏宽、两种会话模式、DAG、Brain 与 Server TUI；见 `ui-final/receipt.json` 和 `chat-evidence/`。
- 发布脚本 **66 项通过**，见 `release-scripts.log`。隔离环境真实 runc、升级、交接、回滚及 **900.54 秒**观察全部通过；53 次连续提交最大受理耗时 **0.319 秒**，见 `smooth-evidence/result.json`。

工作区同时存在其他任务的修改；Rust 全量及最终构建使用本次代码的隔离快照，排除随后进行的项目、Brain 编辑与 Operator 默认提示词改动，保留共享工作区原内容。验证清单见 `accepted-owned-files.json`，范围说明见 `isolation-notes.txt`。起始全量基线未完整完成，不声明基线增量；普通会话容器专属测试随已移除路径替换为宿主执行测试。未提交 Git，未发布生产。

## 相关索引

- [平台能力](../../agent-platform/index.md)
- [Worker](../../../agents/worker/index.md)、[Control](../../../agents/control/index.md)、[Store](../../../agents/store/index.md)、[Web](../../../agents/web/index.md)
