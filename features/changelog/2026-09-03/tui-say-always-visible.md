# TUI Say 永远可见（移除多 subagent 前言隐藏）

Commit: (working-tree)

## 背景

plan 模式下父 agent 先 Say「我要派生 subagent…」、再一轮并发派发多个 explore subagent 时，第 2 个 `SubagentStart` 触发旧 issue #5 机制：`hidden_assistant_idx` 把最近的 Assistant Say 块整体隐藏（渲染 0 行），直到全部 subagent 结束（`SubagentEnd` 计数归零）或 turn 结束（`Done`/`Error`）才一次性恢复。explore 一跑数分钟，期间那句 Say 凭空消失，用户视为内容丢失（必现路径：plan + 多 explore 并发）。隐藏收益（布局聚焦 subagent 面板）远小于「看起来丢内容」的代价，整体移除。

## 变更

- **删除隐藏机制**（tui）：`ChatView.hidden_assistant_idx` 字段、`SubagentStart` 的 `subagents_running == 2` 隐藏起点、`is_withheld()` 及 flatten（`chat.rs` Assistant 分支）与 header 核算（`chat_headers.rs`）两侧跳过守卫、`SubagentEnd`/`Done`/`Error` 三处恢复、`append_reasoning_delta`/`flush_pending_thinking`/ToolStart/ToolEnd echo 四处阶梯插入的索引补偿。Say 在任何时刻渲染行数恒定。
- **保留**：`subagents_running`/`subagents_total` 计数与状态栏徽标、每 subagent 完成即显自身摘要、`SubagentStart` 处 `finalize_assistant()` + `flush_pending_thinking()`（阶梯折叠语义不变）、session crate 零改动。
- 純删除为主（净 -109 行），无新抽象。

## 涉及文件

- `crates/tui/src/chat_types.rs` — 删字段
- `crates/tui/src/chat.rs` — 删起点/恢复/守卫/`is_withheld`（850→800 行）
- `crates/tui/src/chat_headers.rs` / `chat_stream.rs` / `chat_steps.rs` — 删守卫与补偿、注释改写
- `crates/tui/src/subagent_tests.rs` + `chat_tests/subagent.rs`（镜像）— 测试改写
- `crates/tui/src/render_tests/chips.rs` + `chat_tests/line_accounting.rs` — 对齐断言改写（含删 `WithheldPub` 镜像 trait）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 多 subagent 运行中前言始终可见 | `multiple_subagents_keep_preamble_visible` | `subagent_tests.rs` / `chat_tests/subagent.rs` |
| 单 subagent 前言可见 + 摘要即显 | `single_subagent_preamble_visible` | 同上 |
| 失败 sibling 摘要即显 | `failed_subagent_summary_shows_immediately_with_sibling` | 同上 |
| Done 异常收尾计数归零 + 前言可见 | `done_while_subagents_running_resets_count` | 同上 |
| 多 subagent 运行中 header 索引与 flatten 对齐（前言计入行数） | `header_line_indices_aligned_with_flatten_with_preamble_visible` | `render_tests/chips.rs` |

## gate

- `cargo test --workspace` → 3954 passed / 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Impact Surface

- 渲染面：`flatten_with`（chat.rs）与 header 行核算（chat_headers.rs）——Assistant 块不再有 0 行态，行数恒定；hit-rect/选择索引天然对齐（对齐测试已改写断言新契约）。
- 行为面：仅删「第 2 个并发 subagent 起隐藏前言」及其全部恢复/补偿路径；`subagents_running` 计数、状态栏 `↳sub:N` 徽标、单/多 subagent 摘要即显、阶梯折叠语义全部不变。
- 数据面：零改动（无 schema/store/事件变更，`hidden_assistant_idx` 纯 UI 态，不落库、不回放）。

## Related Docs

- [agents/tui](../../agents/tui/index.md) — ChatView 事件接缝与 `1 Turn = n Steps + Say` 契约（未涉及隐藏机制，无需修订）
- [say-closes-turn-transcript-reset-echo.md](say-closes-turn-transcript-reset-echo.md) — Say 关闭 Turn 的配对契约（本次保持不变）
