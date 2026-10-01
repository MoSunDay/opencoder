Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# TUI Server 能力与独立任务

## 变化

- `/agent` 使用 Server 能力库的 Agent/Operator 投影；`self` 创建本地空任务，能力 ID 创建新的远端任务。`@` 文件选择和扩展从 TUI 移除。
- `/task`（`/tasks`、`/t`）恢复本地任务或远端书签，替换当前上下文。远端退出和切换只断开连接，显式停止发送 `interrupt`。
- 首轮准入重试、后续输入和事件重连保持同一执行 ID；问题复用现有弹窗，历史消息、工具结果和压缩中的回合复用现有聊天渲染。
- `opencoder_server` 仅配置 `enabled`、`url`，默认关闭；使用已有 `OPENCODER_SERVER_TOKEN`。远端启动无需本地模型凭据，TUI 本地设置不覆盖 Server 注册的 Codex 包装器。
- 完整展示消息在 SQL 中投影并按字节分块，保留原文、合成标记和用量，排除私有 provider 状态。书签使用已有 Harness JSON，无新增表或环境变量。

## 功能与测试

| 功能 | 测试位置与名称 |
| --- | --- |
| Server 默认关闭、独立合并 URL/开关、配置不含令牌 | [server_config.rs](../../../crates/core/tests/server_config.rs)：`server_connection_defaults_off_and_merges_fields_independently`、`project_server_connection_round_trips_without_a_token_field` |
| 能力目录复用注册库、User 只读及私有定义隔离 | [tui_catalog.rs](../../../crates/control/tests/tui_catalog.rs)：`tui_catalog_reads_library_agent_operator_with_user_role_without_exposing_definitions` |
| 关闭时只有 self、能力种类和摘要筛选 | [agent_menu_catalog.rs](../../../crates/tui/tests/agent_menu_catalog.rs)：`disabled_connection_offers_self_even_with_local_cards`、`capability_picker_filters_kind_and_summary_without_collapsing_targets` |
| `@` 原文进入消息、请求和存储 | [file_mention_flow.rs](../../../crates/tui/tests/file_mention_flow.rs)：`tui_at_paths_are_literal_in_messages_requests_and_store` |
| 同一执行连续输入、不调用本地模型、首轮失败重试 | [remote/tests/mod.rs](../../../crates/tui/src/remote/tests/mod.rs)：`remote_operator_creates_once_continues_same_execution_and_never_calls_local_model`、`failed_first_admission_retries_the_durable_request_with_the_same_id` |
| 首轮各个恢复断点只执行一次、最长执行 ID 和已完成首轮不重复执行 | [initial_input_recovery.rs](../../../crates/worker/tests/initial_input_recovery.rs)：`recovery_admits_once_from_every_pre_execution_fault_point`、`resume_after_completed_first_turn_does_not_call_the_model_again` |
| self/远端上下文隔离、恢复与只断开连接 | [app_task/switch.rs](../../../crates/tui/src/app_task/switch.rs)：`agent_self_detaches_remote_and_creates_an_empty_local_task_then_resumes_only_remote_context`；[remote/tests/mod.rs](../../../crates/tui/src/remote/tests/mod.rs)：`remote_resume_replays_tools_and_user_boundaries_without_recreating_or_mixing_local_context` |
| Server 队列、引导及显式中断 | [remote/tests/mod.rs](../../../crates/tui/src/remote/tests/mod.rs)：`remote_queue_steer_and_explicit_interrupt_target_server_not_local_store` |
| 问题沿用 QuestionHub、断开不跳过问题、代理与凭据隔离 | [protocol.rs](../../../crates/tui/src/remote/tests/protocol.rs)：`credentials_only_go_to_http_and_questions_use_existing_hub_without_skip_on_detach`、`queue_and_steer_admission_use_the_configured_proxy` |
| SSE 分片、游标重连、暂时断线恢复、坏书签拒绝 | [protocol.rs](../../../crates/tui/src/remote/tests/protocol.rs)：`fragmented_sse_handles_utf8_crlf_comments_multiline_data_and_reconnect`、`reconnect_resumes_cursor_and_never_redelivers_frames`、`remote_restore_recovers_after_a_transient_http_failure_without_local_execution`、`incomplete_remote_bookmark_never_falls_back_to_a_local_runner` |
| 完整历史、用量、原文、Unicode 分块和压缩后的当前回合 | [transcript_pagination.rs](../../../crates/store/tests/transcript_pagination.rs)：`bounded_transcript_preserves_display_synthetic_usage_and_utf8`；[protocol.rs](../../../crates/tui/src/remote/tests/protocol.rs)：`chunked_transcript_and_large_tool_payload_are_reassembled_exactly`、`compacted_transcript_uses_display_hides_internal_context_and_preserves_usage`、`compaction_resume_preserves_the_current_streaming_round_without_repeating_a_persisted_answer` |
| 注册 Codex Operator 的目录选择、自动选点、Server 模型与同一线程续会话 | [tui_operator.rs](../../../crates/worker/tests/tui_operator.rs)：`registered_codex_operator_uses_server_settings_auto_placement_and_same_thread` |
| 真终端能力选择、单次回显、任务切换、CLI Codex 恢复和退出断开 | [tui_server.py](../../../scripts/acceptance/tui_server.py)：110×34、72×34 两种窗口 |

## 验证范围

主工作区有并行 DAG 和项目改动。全仓验证使用基线 `7687b5f581254ee6d826d8644789e7d498e761ba` 加本主题代码的独立检出；该结果不覆盖其他任务的混合改动。构建目录也独立，容器用例的 Cargo 子进程使用同一目录。

- TUI 专项：1735 个单测、104 个集成用例通过，0 failed。
- Server 配置、能力目录、完整历史与实际 Server/Worker Codex 包装器专项通过；Codex 两轮均使用 Server 模型，本地模型调用为 0。
- 真 PTY 的两种窗口全部通过，包含带 `--wrap codex` 的远端恢复。
- `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- `cargo build --workspace` 通过。
- 行数检查通过：新增代码文件不超过 400 行，修改文件不超过 800 行；`remote/` 8 个逻辑文件。
- 全量 `cargo test --workspace --no-fail-fast` 完成 434 个套件：5660 passed、1 failed、8 ignored。唯一失败是首轮恢复夹具仍使用旧的 `steer` 和空展示字段；同步为当前 `queue`、展示原文并增加消息原文断言后，恢复套件复验 2 passed、0 failed。按用例去重后的验证结果为 5661 passed、8 项原有 ignored，无未解决失败。
- 8 项原有 ignored 包括 3 项 NFS 挂载、3 项手工 runc、1 项部署设备夹具和 1 项 Chromium 验收；本次未增加跳过项。
- 首次容器 Agent 连续会话用例等待 idle 超时；单独复验两轮通过，随后以 4 个测试线程执行全仓，该套件 11 个用例全部通过，未调整超时或跳过用例。

## 相关索引

- [TUI](../../../agents/tui/index.md)、[Control](../../../agents/control/index.md)、[Worker](../../../agents/worker/index.md)
- [Agent 调度平台](../../agent-platform/index.md#tui-任务入口)、[Harness](../../harness/index.md)
