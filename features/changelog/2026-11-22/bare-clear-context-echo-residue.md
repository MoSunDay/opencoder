# bare `/act_clear_context` 不再复活被打断轮次的 prompt 回显

一句话：轮次运行中手动确认 bare `/act_clear_context`（无尾参）时，
`cancel_running_turn` 刻意保留的 `pending_turn_echo` 残留不再跨过折叠——
fire 路径与 `SteerConsumed` 防御均在 TranscriptReset 重建前清空该记忆，
已取消轮次的 prompt 不会重推为 `User` 块；compound（带尾参）的尾参回显
与跨重建记忆、Shift+Tab 草稿合并路径、轮次自然结束（`Done` 清 echo）等
既有行为全部不变。

改动面：`crates/tui/src/app_loop_actions.rs`（`fire_clear_confirm` 的内联
echo 块替换为对新纯函数的调用）、`crates/tui/src/app_helpers.rs`（新增
`echo_clear_context_fire`：compound 尾参照旧 echo + 记忆，bare 时清空
`pending_turn_echo`——折叠丢弃了产生该 echo 的轮次，重建不得复活）、
`crates/tui/src/chat.rs`（`SteerConsumed` 空 echo 时按
`opencoder_session::split_control_prefix` 判定 bare `ClearContext` 才清，
原始输入取自事件 text 或 `steer_items` 镜像；其他 bare 控制命令不动）。
仅 TUI 回显记忆语义，不触及规则 04/06/07 契约，无公共能力/CLI/API 变更，
不触发 Ontology 同步；未运行全量测试。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| fire echo 契约：compound 记忆 / bare 清残留 | `echo_clear_context_fire_variants` | `crates/tui/src/app_loop_tests/transcript_reset_echo_tests.rs` |
| in-flight bare fire → 重建零 User 块（端到端） | `bare_clear_fire_drops_in_flight_echo_before_rebuild` | 同上 |
| bare clear steer 清 echo（事件 text 路径） | `bare_clear_context_steer_drops_pending_echo` | `crates/tui/src/chat_tests/step_group/turn_boundary.rs` |
| 镜像路径清 / bare 模式切换不清 | `bare_clear_via_mirror_drops_echo_but_mode_switch_keeps_it` | 同上 |

- 定向回归（全部通过，未运行全量测试）：
  - `cargo test -p opencoder-tui --lib transcript_reset_echo` → 5 passed / 0 failed（含既有 compound 尾显回归测试）
  - `cargo test -p opencoder-tui --lib turn_boundary` → 10 passed / 0 failed
  - `cargo test -p opencoder-tui --lib act_clear` → 11 passed / 0 failed
  - `cargo test -p opencoder-tui --lib steer_echo` → 3 passed / 0 failed
  - `cargo test -p opencoder-tui --lib cancel_keep_pending` → 4 passed / 0 failed
  - `cargo test -p opencoder-tui --test compaction_summary_replay` → 7 passed / 0 failed（并存的重建路径改动交叉回归）
  - `cargo test -p opencoder-tui --lib session_ui::compaction_replay_tests` → 4 passed / 0 failed
  - `cargo test -p opencoder-tui --test session_switch_restore` → 3 passed / 0 failed
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
