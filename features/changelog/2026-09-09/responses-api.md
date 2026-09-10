Commit: (working-tree, 基于 0a502305e42a390ba5949ca66211d2d2816512ba)

# Responses API 编码工作流

Provider 增加 `protocol`，显式选择 `responses` 或默认的 `chat_completions`。主模型、small_model、子代理、节点及项目调用在发送时选择端点和协议，使 GPT-5/6 能使用 OpenCoder 的读写、shell、MCP 和多轮会话能力。

请求保持通用 Message 到协议适配层；Responses 使用 `store:false`，保存并回传有序 output、encrypted reasoning、phase 和 call_id。schema v24 增加 nullable `provider_state_json`，兼容旧消息，支持重启、分叉、导出/导入和工具组完整的压缩。推理摘要与 Chat 的 `interleaved_thinking` 开关独立；usage 增加 reasoning tokens。

流处理覆盖文本、推理摘要、拒答、交错工具调用、JSON 网关响应和完成校验。未完成响应及非法工具参数直接失败；中途重试清除未提交输出，CLI/TUI/Web 和子代理同步处理。项目请求追踪记录所选协议的真实请求体。TUI 可编辑协议并区分默认 effort、none、minimal 和其他 effort 值。

配置与功能边界见 [Responses API](../../responses/index.md)。真实 GPT-5/6 与具体网关在线验收按本次约定延期；自动用例均使用本地模拟服务。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 旧配置默认、协议路由 | `protocol_round_trips_and_routes_legacy_and_named_providers` | `crates/core/tests/responses_protocol.rs` |
| 配置保存、删除、非法值 | `protocol_save_load_merge_delete_and_invalid_patch_are_explicit` | `crates/core/tests/responses_protocol.rs` |
| 参数、工具 schema | `maps_parameters_and_preserves_optional_tool_fields` | `crates/llm/tests/responses/request.rs` |
| 作用域隔离、phase、密文 | `state_replay_preserves_phase_ids_and_ciphertext_without_duplicate_blocks` | 同上 |
| 图片和错误工具结果 | `images_and_error_results_keep_their_call_association` | 同上 |
| Unicode、推理、交错工具及 usage | `streams_unicode_reasoning_and_interleaved_calls_without_duplicate_text` | `crates/llm/tests/responses/stream.rs` |
| JSON 响应与拒答 | `json_response_and_refusal_complete_normally` | 同上 |
| 错误与断流不提交 | `terminal_failures_never_commit_tools_or_protocol_state`、`missing_terminal_retries_then_fails_and_regeneration_starts_fresh` | 同上 |
| 混合 provider | `mixed_provider_requests_choose_their_own_protocol` | 同上 |
| 鉴权错误和取消 | `auth_errors_are_immediate_and_dropping_consumer_cancels` | 同上 |
| 429/503 重试、重复事件校验 | `rate_limit_and_server_errors_retry_same_responses_request`、`identical_sequenced_event_is_deduplicated_and_conflicts_are_errors` | `crates/llm/tests/responses/edges.rs` |
| 请求追踪与实际 wire 一致 | `shared_client_trace_body_matches_the_actual_responses_request` | 同上 |
| context 计量不统计密文字节 | `opaque_ciphertext_does_not_inflate_context_estimates_but_reasoning_usage_counts` | 同上 |
| 读→编辑→验证、重启、分叉 | `coding_rounds_persist_replay_and_resume_with_full_provider_state` | `crates/session/tests/responses/lifecycle.rs` |
| 压缩保留工具组与协议状态 | `compaction_preserves_complete_tool_groups_and_replays_retained_state` | 同上 |
| 子代理工具及状态隔离 | `child_tools_and_parent_continuation_use_responses_and_keep_separate_state` | `crates/session/tests/responses/auxiliary.rs` |
| 标题、VERIFY 跨协议和预算 | `title_and_verify_route_to_responses_small_model_with_usable_output_budget` | 同上 |
| 不完整调用不执行、重试重置 | `incomplete_response_never_executes_even_a_complete_tool_item`、`retry_discards_attempt_state_and_emits_reset_before_fresh_text` | `crates/session/tests/responses/failures.rs` |
| v23 升级、bundle、重开 | `legacy_v23_migration_preserves_messages_and_bundle_roundtrips_responses` | `crates/store/tests/responses_state.rs` |
| Provider 表单和 effort | `response_provider_protocol_survives_form_save_reload_and_edit`、`default_none_minimal_and_custom_effort_are_distinct_and_survive_save` | `crates/tui/src/model_menu/tests/responses.rs` |
| 新任务继承会话临时模型及 provider | `new_task_keeps_active_provider_model_and_loads_other_settings` | `crates/tui/src/app_task.rs` |
| TUI 重试保留已完成工具 | `retry_discards_partial_text_and_reasoning_but_preserves_previous_tool_results` | `crates/tui/tests/responses_retry.rs` |
| Web 父/子代理重试 | 两个 reducer 用例 | `crates/web/spa/src/reduce.responses.test.js` |
| HTTP 建客户端并完成编码 | `web_prompt_builds_responses_client_executes_edit_and_persists_sse` | `crates/web/tests/responses_http.rs` |
| CLI 两进程恢复 | `cli_edits_file_and_next_process_replays_responses_state` | `tests/responses_cli.rs` |

## 回归记录

- `cargo clippy --workspace --all-targets -- -D warnings`：通过，零警告。
- `cargo test --workspace --no-fail-fast`：4988 passed / 0 failed / 5 既有 ignored，367 个测试目标。
- `cargo build --workspace`：通过。
- SPA `npm test`：62 个测试文件、501 passed；构建与 `scripts/check-spa-drift.sh` 均通过。
- 行数与 `git diff --check`：通过。未执行真实 GPT-5/6 在线验收。
- [实际验收输出](responses-validation.txt) 保存逐目标 test-result 行和 gate 结果。

初始全仓运行的普通测试累计 4958 passed / 0 failed / 5 ignored，但 doctest 与当时修改中的类型不一致而编译失败，不能作为完整通过基线。未增加 ignored 用例。
