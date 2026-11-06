# 压缩重建去重在途 User 回显（TUI）

一句话：mid-run 压缩/清空重建时，保留尾部若已含当前在途 prompt，旧逻辑会在
`Compaction` 摘要块之前保留一份 `User` 块，又在重建末尾按
`pending_turn_echo` 补一份，导致同一条用户提交出现两次、且旧副本被留在
摘要上方。现在把尾部中匹配在途 prompt 的 `User` 消息延后到摘要块之后重放，
只保留一个续跑边界。

## 变更清单

- `crates/tui/src/session_ui/replay.rs`
  - 新增 `user_echo_text`：与 `replay_one` 同口径（`display` 优先、blocks
    兜底、跳过无 display 的 synthetic 消息）。
  - `replay_into_chat` 保持原公开签名，转调
    `replay_into_chat_inner(..., deferred_echo)`；`rebuild_after_reset`
    把 `pending_turn_echo` 传入。
  - 若折叠转录的最后一个可见 `User` 消息与在途回显一致，主循环跳过它，
    在 `Compaction` 块之后用持久化消息重放（保留图片）；转录中没有该
    prompt 的 clear-context 重建仍直接追加回显。
  - 删除 `rebuild_after_reset` 末尾无条件重推回显的分支。
- `crates/tui/tests/compaction_summary_replay.rs` — 新增回归用例。
- `agents/frontend/tui/index.md` — 同步回放口径。

## 行为口径

- 对外可见顺序仍为「保留尾部 → `Compaction` 摘要 → 续跑」；续跑的
  `User` 边界固定在摘要之后，绝不留在摘要上方。
- 同一条 prompt 既有持久化副本又有 `pending_turn_echo` 时只渲染一次；
  持久化副本优先，以便恢复图片等完整块。
- LLM 上下文顺序不变（`[summary, tail...]`），本次仅调整 TUI 重建顺序。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 尾部在途回显延后到摘要块之后且不重复 | `transcript_reset_defers_retained_in_flight_echo_below_summary` | `crates/tui/tests/compaction_summary_replay.rs` |
| 转录不含 prompt 时仍补回显边界 | `transcript_reset_restores_in_flight_echo_below_rebuilt_view` | `crates/tui/src/app_loop_tests/transcript_reset_echo_tests.rs` |
| 摘要后移 + 幂等去重既有回归 | `resume_replay_renders_summary_head`、`transcript_reset_then_compaction_event_renders_single_block` | `crates/tui/tests/compaction_summary_replay.rs` |

- 定向回归：`cargo test -p opencoder-tui --lib --test
  compaction_summary_replay` → 1817 passed / 0 failed（lib 1814 +
  integration 3）。
- Clippy：`cargo clippy -p opencoder-tui --lib --tests -- -D warnings
  -A clippy::large_enum_variant -A dead_code -A clippy::result_large_err`
  → 零警告。三项 allow 为工作区既有问题，与本次改动无关。
- 测试范围：仅 TUI 压缩回放及直接相邻的 reset 回显链路；未运行全量测试。
