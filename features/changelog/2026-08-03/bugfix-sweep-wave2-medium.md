Commit: (working-tree, pre-initial-commit)

# Bugfix Sweep Wave 2 — 24 Medium-Severity Defects

## 背景
继 Wave 1（7 个 High）之后，本轮修复 24 个 Medium 级别缺陷，覆盖静默错误行为、资源泄漏、部分成功、流式中断等。

## 变更

### llm（4 项）
- **`crates/llm/src/client.rs`**：idle-timeout 戳记移到 send 循环后（Bug1：不再把消费者背压时间计入上游静默）；connect_with_retry 排空 retryable body + 解析 Retry-After（Bug3）；retry 循环顶部检测 `tx.is_closed()` 提前退出（Bug4）
- **`crates/llm/src/tool_call.rs`**：ToolCallDelta 仅在 `started` 后发送，未 start 时缓冲 args（Bug2）
- **`crates/llm/src/retry.rs`**：新增 `backoff_duration(attempt)` 纯函数，供 Retry-After 与指数退避取 max
- **`crates/llm/tests/connect_retry.rs`**（新）：Retry-After 延迟测试 + 消费者取消停止连接循环测试

### core（4 项）
- **`crates/core/src/config.rs`**：`Config::save` 拒绝覆盖损坏文件（Bug1）；`Config::load` 对非对象配置文件 `warn!`（Bug2）；`AgentDefaults` serde default = `"act"`（Bug3）
- **`crates/core/src/config/merge.rs`**：provider headers 改为 extend 合并（Bug4）

### store（2 项）
- **`crates/store/src/libsql_store/events.rs`**：`append_events` 校验批量内 session_id 一致（Bug1）
- **`crates/store/src/import.rs`**：jsonl 导入 append 失败时回滚 session 行（Bug2）
- **`crates/store/tests/store_bugfix_events_import.rs`**（新）：两个集成测试

### web（6 项）
- **`crates/web/src/api.rs`**：DELETE 取消 drain+清 handle（Bug1）；post_agent/post_model get-or-create handle（Bug2）；post_model 先存配置再改 session（Bug3）；list_sessions/messages_response 传播 5xx（Bug4）；update_session 错误不再忽略（Bug5）
- **`crates/web/src/handle.rs`**：flusher 失败 warn 日志（Bug6）

### session（2 项）
- **`crates/session/src/tools/read.rs`**：单行超 MAX_TOKENS 时跳过该行避免无限同偏移重试（Bug1）
- **`crates/session/src/runner/mod.rs`**：bash-timeout dedup 改为仅同命令 dedup（Bug2）

### cli+client（6 项）
- **`crates/cli/src/run.rs`**：--fork 缺 session 报错（Bug1）；--continue 空会话报错（Bug2）；无效 agent 名报错不静默回退（Bug3）
- **`crates/cli/src/client.rs`**：远端流缺 Done 判为截断失败（Bug4）；全局 flag fallback（Bug6）
- **`crates/cli/src/session_cmd.rs`**：workdir canonicalize + 稳定 hash（Bug5）
- **`src/main.rs`**：client 子命令使用全局 session/continue fallback（Bug6）

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| llm: Retry-After 延迟 | `retry_after_header_delays_retry_then_completes` | `crates/llm/tests/connect_retry.rs` |
| llm: 消费者取消停连 | `consumer_drop_stops_connect_loop` | `crates/llm/tests/connect_retry.rs` |
| llm: ToolCallDelta 缓冲 | `apply_buffers_args_then_flushes_once_on_start_without_duplication` | `crates/llm/src/tool_call.rs` |
| core: 损坏配置保护 | `save_handles_corrupt_and_empty_config_files` | `crates/core/src/config.rs` |
| core: 非对象配置告警 | `load_tolerates_non_object_config_file` | `crates/core/src/config.rs` |
| core: AgentDefaults | `agent_defaults_empty_object_deserializes_to_act` | `crates/core/src/config.rs` |
| core: headers 合并 | `merge_into_appends_provider_headers` | `crates/core/src/config/merge.rs` |
| store: 混 session_id 拒绝 | `append_events_rejects_mixed_session_ids` | `crates/store/tests/store_bugfix_events_import.rs` |
| store: 导入回滚 | `import_jsonl_failure_rolls_back_empty_session` | `crates/store/tests/store_bugfix_events_import.rs` |
| session: 超长行跳过 | `test_oversized_first_line_skipped_no_loop` | `crates/session/src/tools/read.rs` |
| session: dedup 同命令 | `different_commands_not_deduped` | `crates/session/src/runner/mod.rs` |
| cli: --fork 校验 | `fork_without_session_or_continue_errors` | `crates/cli/src/run.rs` |
| cli: --continue 校验 | `continue_with_no_sessions_errors` | `crates/cli/src/run.rs` |
| cli: agent 名校验 | `reapply_resume_agent_rejects_unknown_name` | `crates/cli/src/run.rs` |
| cli: canonicalize | `data_dir_for_canonicalizes_symlinks_and_trailing_slash` | `crates/cli/src/session_cmd.rs` |
| client: 全局 fallback | `client_session_flags_fall_back_to_globals` | `crates/cli/src/client.rs` |

- 全量回归：`cargo test --workspace` → 全绿（1716 passed, 0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：所有文件 ≤800 行

## Impact Surface
- llm：流式不再因消费者背压误判超时；429/5xx 连接复用+遵守 Retry-After；消费者取消不再浪费连接
- core：损坏配置不再被覆盖；headers 配置可叠加；agent 默认值一致
- store：混 session 批量被拒绝；导入失败可重试
- web：DELETE 不泄漏 drain/handle；override 不丢失；DB 错误传播 5xx
- session：read 不再无限循环；dedup 不隐 PID
- cli：所有静默错误行为改为显式报错；workdir hash 稳定

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/llm](../../agents/llm/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/core](../../agents/core/index.md)
- [agents/store](../../agents/store/index.md)
