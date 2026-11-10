# 工具调用配对的 wire 级守卫（请求兜底 + replay 根因）

一句话：为 Chat Completions 请求体补上「assistant 的每个 `tool_calls` id 必须被
紧邻的 `role:"tool"` 消息逐一应答」的 wire 级不变量——`ChatRequest::to_body`
出口新增纯函数单遍 sanitizer（丢弃孤儿 tool 消息、剥离未答 call、移除变空的
assistant，合法 transcript 字节级不变），并把 `replay_cancelled_tasks` 的
重放可见性守卫统一到 `task_tool_use_visible`（根因侧不再新增污染）。

## 行为口径

- **A. wire 级 sanitizer**（`crates/llm/src/sanitize.rs`，纯函数、单遍、无重排）：
  `sanitize_tool_messages(Vec<Value>) -> (Vec<Value>, SanitizeReport)`，三个
  usize 计数（`dropped_tool_messages` / `stripped_tool_calls` /
  `dropped_assistant_messages`）+ `is_clean()`。只做三件事：
  1. 丢弃配不上的 `tool` 消息——id 缺失/非字符串、不属于当前 assistant
     `tool_calls`、重复应答、或出现在任何非 tool 消息之后（group 已关闭）；
  2. group 关闭时（下一条非 tool 消息或输入结束）从 `tool_calls` 剥离无结果 id；
  3. 剥离后既无 `tool_calls` 又无 content（null/空）/reasoning 的 assistant
     整条移除（保留文本时删除空的 `tool_calls` 键——空数组同样违约）。
  合法 transcript（system/user/并行连续结果/图片 user 消息在整批结果之后）
  原样通过、字节级不变。接线点为 `to_body`（唯一漏斗：真实请求、stream trace、
  重试复用同一 body）；非 clean 时打一条 `tracing::warn!` 带计数。Responses /
  embeddings 不经此路径。
- **B. 根因守卫**（`crates/session/src/resume.rs` `replay_cancelled_tasks`）：
  原守卫只在 handoff 边界后检查可见性，压缩边界（`summary_seq` 裁剪）下放行
  了 dispatch 已被摘要裁掉的 Cancelled 任务，重放回填的 `tool_result` 会应答
  provider 看不见的 `tool_use`——正是 wire 级孤儿。现无条件要求 dispatch 的
  `tool_use` 在当前（可能已裁剪的）内存 transcript 里可见，与
  `filter_replay_candidates` 跨进程守卫复用同一谓词
  `dangling_tools::task_tool_use_visible`。
- **取舍**：drop 而非重排——罕见损坏态下丢弃的是已摘要的重复结果或错误占位
  符，换取可一眼验证的实现；会话不再 400，模型需要时可重新调用工具。不重写
  store 历史，存量脏会话靠请求时兜底恢复可用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| sanitizer 纯函数（合法原样/缺 id/非字符串 id/重复/先于调用/计数/幂等） | `sanitize::tests::*`（6 项） | `crates/llm/src/sanitize.rs` |
| 合法 transcript 经 `to_body` 原样通过 | `legal_transcript_passes_through_unchanged` | `crates/llm/tests/tool_pairing_wire.rs` |
| 压缩边界头部孤儿 tool 丢弃 | `orphan_tool_at_head_is_dropped` | 同上 |
| user 夹在中间 → 后续 tool 丢弃 + call 剥离 | `user_between_calls_and_results_strips_unanswered_call` | 同上 |
| 图片 user 消息场景（图片保留、后续 tool 丢弃） | `image_user_message_between_batches_is_preserved` | 同上 |
| 未答 call 剥离、文本 assistant 保留 | `unanswered_call_on_text_assistant_stripped_message_kept` | 同上 |
| tool-call-only 无结果 → 整条移除 | `tool_call_only_assistant_without_results_removed` | 同上 |
| 重复结果只留一条、结果先于调用丢弃 | `duplicate_result_kept_once_and_early_result_dropped` | 同上 |
| replay 可见性守卫（摘要头内 dispatch+result 不重放） | `replay_skips_cancelled_task_answered_in_summarized_head` | `crates/session/tests/replay_skips_answered_cancelled.rs` |

## 实测记录

- `cargo test -p opencoder-llm --lib sanitize` → 6 passed / 0 failed
- `cargo test -p opencoder-llm --test tool_pairing_wire` → 7 passed / 0 failed
- `cargo test -p opencoder-llm --test lower_messages --test request_body` → 16 + 9 passed / 0 failed（`to_body` 合法输入不变的金丝雀）
- `cargo test -p opencoder-session --test replay_skips_answered_cancelled --test replay_cancelled_empty_guard --test resume_replay` → 2 + 1 + 9 passed / 0 failed
- `cargo test -p opencoder-session --test question_tool --test cache_salt --test tool_image_contract` → 3 + 7 + 1 passed / 0 failed（`to_body` 消费方金丝雀）
- `cargo clippy -p opencoder-llm -p opencoder-session --all-targets -- -D warnings` → 零警告
- `cargo fmt -p opencoder-llm -p opencoder-session -- --check` → 通过（`--all --check` 在未触及的 crate 有既有 diff，非本次引入）
- 测试范围：本次修改及其直接影响链路；未运行全量测试。真实模型端到端需
  API key，不在自动化门禁内；`to_body()` 输出即线上 wire JSON，同一不变量
  已被上述测试覆盖。
