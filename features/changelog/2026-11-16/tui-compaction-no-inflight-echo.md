# 压缩重建丢弃在途 User 回显（TUI）

一句话：mid-run 压缩（自动或手动）触发的 `TranscriptReset` 重建不再回显
`❯ User: <original prompt>` —— 保留尾部副本与 `pending_turn_echo` 均不重放，
视图直接从 `Compaction` 摘要块继续；clear-context 重建仍保留在途 `User` 边界。

## 背景与结论

本变更**反转**了 [2026-11-06 的压缩回显口径](../2026-11-06/tui-compaction-inflight-echo.md)：
旧行为把尾部匹配的在途 `User` 延后到摘要块下方重放（避免摘要上方旧副本与
末尾补推重复）；新行为在压缩重建中直接丢弃该回显，只保留 `Compaction` 摘要。
2026-11-06 的历史记录保持原样，不回改。

## 变更清单

- `crates/tui/src/session_ui/replay.rs`
  - 新增 `has_compaction_summary`：识别折叠转录中的 synthetic 摘要头。
  - `replay_into_chat_inner` 计算 `compacting`；压缩重建跳过尾部副本的延后
    重放与 `pending_turn_echo` 补推。`deferred_user_idx` 的“最后一个可见且
    文本匹配的 `User`”匹配逻辑与主循环跳过逻辑保持不变。
  - `rebuild_after_reset` 在压缩重建后把 `pending_turn_echo` 清为 `None`，
    同轮后续 reset 不会复活回显；`turn_block_start` 仍落在摘要/尾部块之后。
- `crates/tui/src/chat_types.rs`、`crates/tui/src/compaction_block.rs`、
  `crates/tui/src/chat_tests/compaction_state.rs` — 同步注释口径。
- `crates/tui/tests/compaction_summary_replay.rs` — 重写/新增回归用例。
- `agents/frontend/tui/index.md` — 同步 `session_ui/replay.rs` 回放说明。

## 行为口径

- **压缩（自动 / 手动）**：重建视图只含摘要块；在途 prompt 的尾部持久化副本
  与 `pending_turn_echo` 都不再渲染，重建后 `pending_turn_echo` 为 `None`。
- **clear-context**：转录未含该 prompt 时仍在重建块之后补回显边界；尾部若
  已含则重放持久化副本（图片等完整块保留），阶梯不会并入 reset 前的轮次。
- **resume 回放**与发给 LLM 的 `[summary, tail...]` 上下文顺序不变。
- `tail_turns > 0` 保留的历史轮次仍照常显示在摘要块上方。
- 只丢弃“最后一个可见且文本匹配”的在途 `User`；更早的同文本历史保留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 压缩重建只渲染摘要、不补回显，最终事件不叠孪生块 | `transcript_reset_then_compaction_event_renders_single_block` | `crates/tui/tests/compaction_summary_replay.rs` |
| 尾部在途副本被丢弃 | `transcript_reset_drops_retained_in_flight_echo` | 同上 |
| 保留的历史尾部轮次仍在摘要上方，在途 prompt 不回显 | `transcript_reset_keeps_prior_tail_turns_but_drops_in_flight_echo` | 同上 |
| 无尾部 prompt 的压缩重建不补回显 | `transcript_reset_without_tail_prompt_does_not_push_echo` | 同上 |
| clear-context 重建保留在途回显边界 | `clear_context_reset_keeps_in_flight_echo` | 同上 |
| resume 摘要头回放（未改动） | `resume_replay_renders_summary_head` | 同上 |

## 实测

定向测试（实际执行）：

```text
cargo test -p opencoder-tui --test compaction_summary_replay   # 6 passed / 0 failed
cargo test -p opencoder-tui --lib transcript_reset_echo        # 2 passed / 0 failed（1825 filtered out）
cargo test -p opencoder-tui --lib turn_boundary                # 8 passed / 0 failed（1819 filtered out）
cargo test -p opencoder-tui --lib compaction_state             # 11 passed / 0 failed（1816 filtered out）
cargo test -p opencoder-tui --lib compaction_replay            # 4 passed / 0 failed（1823 filtered out）
cargo test -p opencoder-tui --lib handoff_card                 # 3 passed / 0 failed（1824 filtered out）
```

- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 通过
  （exit 0，零警告）；未使用任何代码级 `#[allow]`，也不需要历史 allow 组合。
- 格式：`cargo fmt -p opencoder-tui -- --check` 在本次未触及的 11 个既有文件上
  报告格式漂移（`app_display.rs`、`app_helpers.rs`、`app_loop.rs`、
  `app_loop_actions.rs`、`act_clear_busy.rs`、`stale_turn_done_tests.rs`、
  `app_submit.rs`、`idle_rekick.rs`、`key_handler_cancel_busy_tests.rs`、
  `worker/tests_reload.rs`、`tests/agent_mention_flow.rs`）；本次触及文件
  单独执行 `rustfmt --check --edition 2021` 通过。为遵守“不改无关文件”，
  未对这些既有漂移做格式化。
- `git diff --check` → 通过（无输出）。
- **未运行全量测试**：仅运行上述 TUI 压缩回放、reset 回显、轮次边界及相邻
  定向用例。
