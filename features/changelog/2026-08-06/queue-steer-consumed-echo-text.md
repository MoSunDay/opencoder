Commit: (working-tree, pre-initial-commit)

# QueueConsumed / SteerConsumed 事件携带 prompt 文本

## 背景
`SessionEvent::QueueConsumed` / `SteerConsumed` 此前只携带 `seq`，不携带 prompt 文本。
TUI 依赖本地 `queue_items` / `steer_items` 镜像查文本回显；但 **Web 前端**和 **CLI headless** 没有
本地镜像，prompt 文本只在 turn 结束后 `done` → `/messages` 全量重建时才出现——表现为
「queued 输入的回显出现在输出之后」。

根本原因：`steer.rs:184` 先 emit `QueueConsumed`，`record_compound` 之后才持久化消息，
所以收到事件那一刻消息尚未落库——无状态客户端无法靠 fetch 拿到文本。

## 变更

### session: 事件增加 `text` 字段
- **`crates/session/src/runner/event.rs`**：`QueueConsumed` / `SteerConsumed` 各增加
  `#[serde(default)] text: String`（向后兼容旧持久化事件，缺省为空）。
  `sse_data` 输出 `{"seq","text"}`；`from_sse` 解析 `text`（缺省空串）。
- **`crates/session/src/runner/steer.rs`**：`drain_one_queued` emit 时带上 `text: q.clone()`。
- **`crates/session/src/runner/mod.rs`**：`run_loop` steer-promote 循环 emit 时带上 `text: p.clone()`。

### tui: 升级为 `user:` 风格完整回显
- **`crates/tui/src/app_loop.rs`**：`QueueConsumed` handler 改为用事件携带的 `text` 回显
  （空时回退本地镜像），输出 `user: {text}` + 空行（与 `push_user` 一致），保留原色区分。
- **`crates/tui/src/chat.rs`**：`SteerConsumed` handler 同上。

### cli: headless 回显 prompt
- **`crates/cli/src/display.rs`**：`QueueConsumed` / `SteerConsumed` arm 改为打印 `user: {text}`。

### web: 新增 live 回显监听器
- **`crates/web/src/assets/render.js`**：新增 `queue_consumed` / `steer_consumed` SSE 监听器，
  在激活瞬间插入 `user:` 消息 div（并 null-out `curAssistant` 确保后续输出在新块之后），
  不再等 `done` → `load()`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| QueueConsumed 序列化携带 text | queue_consumed_carries_text_through_sse | crates/session/src/runner/event.rs |
| SteerConsumed 序列化携带 text | steer_consumed_carries_text_through_sse | crates/session/src/runner/event.rs |
| 旧 payload 无 text 向后兼容 (queue) | queue_consumed_without_text_field_is_backward_compatible | crates/session/src/runner/event.rs |
| 旧 payload 无 text 向后兼容 (steer) | steer_consumed_without_text_field_is_backward_compatible | crates/session/src/runner/event.rs |
| queue 激活回显先于 TextDelta | queue_consumed_carries_text_and_precedes_output | crates/session/tests/queue_echo.rs |
| /plan 复合命令 raw text 保留 | queue_consumed_compound_carries_raw_text（2026-09-01 收敛为 model-facing echo 后更名 `queue_consumed_compound_carries_tail_text`，断言改为仅尾参） | crates/session/tests/queue_echo.rs |
| TUI QueueConsumed 回显 user: | fold_queue_consumed_echoes_marker / fold_queue_consumed_unknown_seq_is_noop | crates/tui/src/app_loop_tests/mod.rs |
| TUI SteerConsumed 回显 user: | steer_consumed_echoes_marker_and_drops_row / steer_consumed_unknown_seq_is_noop | crates/tui/src/chat_tests/steer_echo.rs |
| plan_card SteerConsumed 回显 | steer_consumed_echoes_marker_and_drops_entry | crates/tui/src/chat_tests/plan_card.rs |
| SSE 全 variant 回放保真 | replay_kind_matches_live_kind_for_all_variants | crates/web/tests/replay_fidelity.rs |
| SteerConsumed 模式兼容 (..) | steer_consumed_carries_pk_seq_not_admitted_seq 等 | crates/session/tests/steer_followup.rs |
