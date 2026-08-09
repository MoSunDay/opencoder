# 逻辑错误清扫：11 个静默潜伏 bug 修复

## 背景

构建清洁、2248 个测试全绿（含并发 notepad-scroll 提交），但代码审查发现 11 个潜在的正确性 / 安全漏洞正在静默
潜伏（无失败测试覆盖）。本轮集中修复，每个修复附单元 / 集成测试。无架构变更，
均为小而精准的编辑（1–15 行）。

## 变更

### 🔴 高危

**Bug 1 — 计划模式写入防护绕过**（`crates/session/src/bash_guard.rs`）
- `split_segments` 未将裸 `&`（shell 后台操作符）与 `\n`（换行）视为分隔符。计划
  模式下 `echo ok & rm -rf /tmp/x` 或 `echo ok\nrm ...` 产生单个以 `echo` 起首的片段
  → 归类只读 → 变异部分未检查 → 核心安全边界被静默绕过。
- 修复：单字符分隔符分支增加 `|| c == '&' || c == '\n'`。两字符 `&&`/`||` 检查在
  同一轮迭代更早处（带 `continue`）已捕获，裸 `&` 仅在非 `&&` 时到达此分支。

### 🟡 中危

**Bug 2 — drain-pending 重启丢弃 `start_turn` 返回值**（`crates/tui/src/app_loop.rs`）
- `fold_ui_events` 中 `TurnDone`/`drain_pending` 重启分支未检查 `start_turn(...)` 的
  `bool` 返回值（其余所有调用点都检查）。worker 已死时 UI 进入永久 running 加载态。
- 修复：匹配其余调用点 `if !start_turn(...).await { worker_dead(chat); return LoopFlow::Quit; }`。

**Bug 3 — SwallowTail Esc 守卫不清截止时间 → 100% CPU 忙循环**（`crates/tui/src/input.rs`）
- `flush_expired()` 仅处理 `Holding` 状态。Esc 守卫处于 `SwallowTail`（消耗拆分 CSI
  序列）且窗口过期时，状态永不变 `Idle`、`deadline` 永不清除 → `poll_timeout` 返回
  `Duration::ZERO` → 输入泵每轮立即唤醒 → 100% CPU 忙循环。
- 修复：`flush_expired` 改为 `match` 覆盖三态；过期的 `SwallowTail` 回到 `Idle`、清
  `deadline`、不 emit（被吞的 Esc 无需提交）。

**Bug 4 — 重复 `LlmEvent::Error` + 双重前缀**（`crates/llm/src/client.rs`）
- `ChunkError`/`IdleTimeout` 耗尽重试预算时，降级分支既向 `tx` 发
  `LlmEvent::Error("stream failed: …")` 又 `return Err(anyhow!("stream failed: …"))`；
  外层 `chat_stream` 捕获该 `Err` 后再发一次
  `LlmEvent::Error(format!("stream failed: {e:#}"))` → 消费者收到两个 Error，且第二条
  把已带 `stream failed:` 前缀的消息再前缀一次（"stream failed: stream failed: …"）。
- 修复：耗尽路径只返回 `Err`、不在降级分支预先发射，由外层单次发射，保证恰好一个、
  无双重前缀。

**Bug 5 — 工具专用 assistant 轮 `content` 为空串而非 `null`**（`crates/llm/src/message.rs`）
- `push_assistant` 在 `text.is_empty()` 时把 `content` 设为 `Value::String(String::new())`
  （`""`）。OpenAI 规范要求：当 assistant 消息只有 `tool_calls`、无文本时，`content`
  必须为 `null`；空串会被严格 provider / proxy 以 HTTP 400 拒绝。
- 修复：`text.is_empty() && !tool_calls.is_empty()` 时发 `Value::Null`；纯文本轮次仍为字符串。

**Bug 6 — 裸 steer（仅切模式）仍发起 LLM 轮**（`crates/session/src/runner/mod.rs`）
- 控制命令（`/act`、`/plan`）已有无 LLM 短路，但带模式切换的"裸 steer"（无伴随
  用户内容）未走同一短路，仍触发完整 LLM turn — 白耗 token 且产生无意义回复。
- 修复：与控制命令一致，裸 steer 短路返回，零 LLM 调用。

**Bug 7 — SSE 解码遇无效首字节永久卡死**（`crates/llm/src/sse.rs`）
- `drain()` 中 `from_utf8` 失败且 `valid_up_to() == 0`（缓冲区以无效字节起首，如游离
  的 continuation byte `0x80`）时直接 `return Vec::new()`，既不消费该字节也不推进
  `self.buf` → 后续每轮 `drain` 仍命中同一无效字节 → 流永久卡死、再无事件解码。
- 修复：跳过（丢弃）无效首字节后继续解码，而非原样保留。

### 🟢 低危

**Bug 8 — SSE 去重 `seen` 集合恒空**（`crates/web/src/api.rs`）
- `baseline = last_event_seq()` 在 `events_after` 查询**之后**运行，故
  `baseline >= max(persisted.seq)` → `seq > baseline` 过滤器恒假 → 内容去重（重叠
  窗口的二级去重）是死代码。修复：将 `baseline` 查询移到 `events_after` 之前。

**Bug 9 — `post_skill` 缺会话存在检查**（`crates/web/src/api_ops.rs`）
- `post_skill` 缺 `get_session` 存在检查（`post_compact`/`post_handoff` 均有），对不
  存在的会话返回错误成功 `{ok:true}`。修复：增加与 `post_compact` 一致的存在守卫。

**Bug 10 — `LibsqlStore.load_after` 绕过负偏移 clamp**（`crates/store/src/libsql_store/messages.rs`）
- `load_after` 直接把 `skip_count` 传入 SQL `OFFSET ?`，绕过 trait 默认的
  `clamp(0, i64::MAX)`。负偏移会到达 `OFFSET`。修复：`let skip_count = skip_count.max(0);`。

**Bug 11 — `parse_usage` 省略 `total_tokens` 时报 0**（`crates/llm/src/client.rs`）
- `total_tokens` 用 `unwrap_or_default()`（0），token 统计静默归零。修复：
  `.filter(|&t| t != 0).unwrap_or(input_tokens + output_tokens)`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 计划模式检测 `&` 后写入 | `classify_detects_write_after_bare_ampersand` | `crates/session/src/bash_guard_tests.rs` |
| 计划模式检测 `\n` 后写入 | `classify_detects_write_after_newline` | `crates/session/src/bash_guard_tests.rs` |
| 两只读命令以 `&` 连接仍只读 | `classify_bare_ampersand_between_readonly_stays_readonly` | `crates/session/src/bash_guard_tests.rs` |
| 过期 SwallowTail 清截止时间 | `flush_expired_clears_swallow_tail_when_window_passes` | `crates/tui/src/input.rs` |
| 过期 Holding 仍提交 Esc | `flush_expired_commits_holding_when_expired` | `crates/tui/src/input.rs` |
| 未过期守卫返回 None 不变 | `flush_expired_returns_none_when_not_expired` | `crates/tui/src/input.rs` |
| drain 重启遇 worker 已死 → Quit | `drain_pending_restart_with_dead_worker_quits` | `crates/tui/src/app_loop_bugfix_tests.rs` |
| 耗尽路径恰好一个 Error | `retry_exhaustion_emits_single_non_doubled_error` | `crates/llm/tests/stream_retry.rs` |
| 工具专用轮 content 为 null | `assistant_tool_only_content_is_null` | `crates/llm/tests/lower_messages.rs` |
| 多轮工具专用均 null | `multi_turn_tool_only_messages_all_have_null_content` | `crates/llm/tests/lower_messages.rs` |
| 文本+工具仍为字符串 | `assistant_with_both_text_and_tool_calls_keeps_string_content` | `crates/llm/tests/lower_messages.rs` |
| 裸 steer 切模式零 LLM 调用 | `bare_steer_switches_mode_with_no_llm_call` | `crates/session/tests/bare_steer_short_circuit.rs` |
| 跳过无效首字节解码帧 | `drain_skips_invalid_leading_byte` | `crates/llm/src/sse.rs` |
| 连续无效首字节 | `drain_skips_run_of_invalid_leading_bytes` | `crates/llm/src/sse.rs` |
| 重叠窗口事件去重一次 | `overlap_window_event_is_deduped_once` | `crates/web/tests/sse_overlap_dedup.rs` |
| 不存在会话 skill 返回 404 | `skill_nonexistent_returns_404` | `crates/web/tests/web_api_ops.rs` |
| 负偏移返回全部消息 | `load_messages_after_negative_offset_returns_all` | `crates/store/tests/load_messages_after.rs` |
| 省略 total 回退 input+output | `parse_usage_derives_total_when_omitted` | `crates/llm/src/client_tests.rs` |
| 显式 total 保留 | `parse_usage_preserves_explicit_total` | `crates/llm/src/client_tests.rs` |
| 显式 0 total 回退 | `parse_usage_derives_total_when_explicit_zero` | `crates/llm/src/client_tests.rs` |

- 全量回归：`cargo test --workspace -- --test-threads=1` → **2268 passed / 0 failed**（隔离验证：仅本提交，不含 runtime/TUI 代码；baseline 2248 + 本轮 20）（本轮新增 20 个测试）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 构建：`cargo build --workspace` → 零错误
