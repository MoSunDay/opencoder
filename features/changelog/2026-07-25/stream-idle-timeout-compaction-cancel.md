Commit: (working-tree, pre-initial-commit)

# feat(session): 可配置的流式空闲超时 + 任务超时，压缩摘要可中断

## 背景

两层健壮性缺口：

1. **LLM 流假死**：上游用 SSE keep-alive 注释帧保活却不发内容时，旧的固定
   120s 空闲超时是硬编码的，无法调；压缩摘要流（`summarize`）甚至**完全没有**
   中断/空闲守卫——双击 Esc 或 web interrupt 期间，压缩会一直阻塞 runner 直到
   摘要跑完。
2. **子任务无上界**：`task` subagent 可无限挂起，无超时护栏。

本轮把空闲/任务超时**配置化**，并让压缩摘要复用 LLM 调用的中断 + 空闲守卫范式。

## 变更

### 配置（`crates/core/src/config.rs`）
- `Config` 增两个可选字段（serde `default = None`，向后兼容）：
  - `stream_idle_timeout_secs: Option<u64>`（默认 120）——流式调用无事件即判
    stalled 并中止，独立于 HTTP `read_timeout`，专抓「连接活着、只发 keep-alive、
    不给内容」的假死。
  - `task_timeout_secs: Option<u64>`（默认 1800 / 30 min）——`task` subagent
    墙钟上限，防无限挂起。
- 访问器 `stream_idle_timeout()` / `task_timeout()` 返回 `Duration`（config.rs:381-387）。
- `merge_into` 增两字段覆盖（config.rs:792-796）。

### 中断原语（`crates/session/src/runner/steer.rs`）
- `await_cancel(session)`（steer.rs:7，`pub(crate)`）：监听双击 Esc / web
  interrupt，被取消信号唤醒即 resolve。在 `runner/mod.rs:30` re-export，供
  LLM 调用 / 工具执行 / 压缩共用。

### LLM 调用守卫（`crates/session/src/runner/llm_call.rs`）
- `run_one_llm_call` 的 select 循环加 `cancel_fut = await_cancel(session)` +
  每轮重建的 `idle = sleep(stream_idle_timeout())`（llm_call.rs:65-66）：cancel
  → 状态 `interrupted` 中止；idle → 状态 `stream idle` 报错。SSE keep-alive 注释
  不经该 channel，故「只发 keep-alive」被判为 idle。

### 压缩可中断（`crates/session/src/compaction.rs`）
- `summarize` 复用同一守卫（compaction.rs:248-274）：cancel → `interrupted` 且
  `Err("cancelled")`；idle → `stream idle` 报错。**关键**：`compact` 只在
  `summarize` 返回 Ok **之后**才改写 `session.messages`，故取消时直接 abandon
  摘要、转录原封不动。

### 子任务超时（`crates/session/src/runner/execute.rs`）
- `task` 执行包 `task_timeout()` 墙钟上限（execute.rs:49），与 cancel 守卫并列。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 空闲超时默认 120s | `stream_idle_timeout_defaults_to_120s` | core/src/config.rs |
| 空闲超时可配 | `stream_idle_timeout_is_configurable` | core/src/config.rs |
| 任务超时默认 1800s | `task_timeout_defaults_to_1800s` | core/src/config.rs |
| 任务超时可配 | `task_timeout_is_configurable` | core/src/config.rs |
| 压缩 honor cancel 且不破坏转录 | `compact_honors_cancel_and_leaves_messages_intact` | session/src/compaction.rs |

- 全量回归：`cargo test --workspace`（隔离 target）→ **1057 passed; 0 failed**。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。

## Impact Surface
- **用户**：双击 Esc / web interrupt 能**立即**打断压缩摘要（此前会阻塞到摘要完成）；
  stalled 的 LLM 流在可配的空闲窗口后被中止并报错，不再无限挂起。
- **运维**：`stream_idle_timeout_secs` / `task_timeout_secs` 可在配置文件/JSON 覆盖。
- **向后兼容**：两字段 `Option`+serde default，旧配置无需改动；默认值与原硬编码一致（120s）。
- **不影响** store / web 协议；纯 session/core 层守卫增强。

## Related Docs
- [agents/session](../../agents/session/index.md)
- 守卫范式来源：`run_one_llm_call`（`crates/session/src/runner/llm_call.rs`）
