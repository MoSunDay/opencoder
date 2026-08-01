# fix(session): autopilot 迭代 ≥2 移交边界错记 + 阶段错误缺收尾 + 测试修复

## Summary

autopilot 深度 review 后的一轮修复（另含 TUI 测试拆分遗留问题）：

1. **移交边界 store 计数错记（HIGH）**：`handoff()` /
   `SessionState::store_message_count()` 只把「内存头部可能有一条压缩摘要
   （`summary_seq`）不在 store」计入公式，未计「plan→act handoff / clear-context
   标记（`handoff_seq`）同样是不在 store 的合成头部」。autopilot 迭代 ≥2 时，
   第二次 ACT 阶段 handoff 会把 `handoff_seq` 记成 `messages.len()` 而非真实
   store 消息数（少记 N-1）——resume 按错误下标 trim，把本应丢弃的 plan 模式
   历史重新塞回上下文。同理 `/act_clear_context`（handoff 之后）与
   handoff 之后的压缩（`compaction.rs` 的 `head_store_msgs` 多记 1）受影响。
   三处统一为 `skip + len - 1`（有合成头部时）。
2. **drive 阶段错误缺收尾（MEDIUM）**：`drive` 的 PLAN/ACT 阶段 `?` 上抛错误时
   跳过 `finish()`——review skill 残留到下一用户 turn，且不发终止 `Done`。
   现在错误路径先走 `finish`（清 skill + emit `Done`）再原样上抛。
3. **resume.rs 死语句**：`let _ = &mut s;` 及其暴露的 `mut` 移除。
4. **TUI 测试问题**：`parent_with_long_subagent` 在鼠标测试拆分时被复制两份
   （`mouse_clip_tests` / `mouse_scroll_tests`），提升到 `mouse_helpers.rs`
   共享，并修正陈旧注释；`mouse_wheel_tests` / `key_handler_queue_scroll_tests`
   的「下限钳制」测试先手动置 0 再递减（只测 0→0，空转），改为真实 1→0 跨越。

## Changes

### `crates/session/src/lib.rs`
- `SessionState::store_message_count`：`summary_seq` 或 `handoff_seq` 任一有值时
  头部为不在 store 的合成消息，store 计数 = `skip + len - 1`。

### `crates/session/src/plan_handoff.rs`
- `handoff()` 的 `store_msg_count` 同公式修正（注释指向 `store_message_count`）。

### `crates/session/src/compaction.rs`
- `head_store_msgs`：`handoff_seq` 有值（合成 handoff 头部）时同样按 `split - 1`
  计 store 消息数。

### `crates/session/src/autopilot/mod.rs`
- `drive`：PLAN/ACT 阶段错误先 `finish`（清 skill + `Done`）再上抛 `Err`。

### `crates/session/src/resume.rs`
- 删除死语句 `let _ = &mut s;`，`SessionState` 绑定去 `mut`。

### `crates/tui/src/app_helpers_tests/`（`mouse_helpers.rs` / `mouse_clip_tests.rs` / `mouse_scroll_tests.rs` / `mouse_wheel_tests.rs`）+ `key_handler_queue_scroll_tests.rs`
- 测试夹具去重 + 下限钳制测试真实化。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 迭代 ≥2 移交边界 = 真实 store 计数 | `drive_iteration_two_persists_true_store_handoff_boundary` | `session/tests/autopilot.rs` |
| 阶段错误清 skill + emit Done + 上抛 | `drive_phase_error_clears_skill_and_emits_done` | 同上 |
| 既有 autopilot 全量回归 | `verify_*` / `drive_*` / `act_phase_*` | 同上 |
| 控制命令 / 压缩 / resume 回归 | `control_cmd.rs` / `compaction_and_model.rs` / `handoff_resume.rs` | `session/tests/` |
| wheel-down 真实 1→0 下限 | `wheel_down_in_queue_panel_returns_toward_newest` | `tui/src/app_helpers_tests/mouse_wheel_tests.rs` |
| Shift+PageDown 真实 1→0 下限 | `shift_page_down_floors_at_zero` | `tui/src/key_handler_queue_scroll_tests.rs` |
| 子代理视图复制 / 滚动（共享夹具） | `subagent_view_drag_copies_child_text` 等 | `tui/src/app_helpers_tests/` |

- 全量回归：`cargo test --workspace` → 1586 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
