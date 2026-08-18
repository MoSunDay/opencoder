Commit: (working-tree, post-f801b27)

# TUI 已提交输入不再被吞：cancel/Error 保留 pending、admitter 竞态补行、steer 武装 plan 交接、handoff 降级不丢输入

## 背景

四个「用户已提交的输入被静默丢弃」类缺陷：

1. 双击 Esc 硬中止（`KeyAction::Cancel`）曾把 pending steer/queue 行从 store 删除并清空镜像（`clear_pending_inputs`）——取消一个 run 连带吞掉所有排队输入。
2. worker `Error` 事件曾直接 `chat.steer_items.clear()` 且无恢复——镜像与 store 脱钩，用户看不到仍 pending 的行。
3. queue admitter 竞态：`Done` 触发的权威镜像重建可在 actor completion 落地前覆盖乐观 temp 行，`reconcile_ok` 无从补回——排队输入从面板消失（store 里仍在）。
4. 键盘 Enter-steer 路径缺少 requirement 记账：plan 模式下补充需求的 steer 不武装 `plan_submitted`，Shift+Tab plan→act 交接感知不到该需求。
5. `worker::handoff_run_prompt` 已定义未接线：`SwitchAndStart` 溯源门失败时注释声称「捕获输入作为普通 act prompt 提交」，实际跑空 turn——输入框文本被静默丢弃。

## 变更

### cancel 保留 pending（对齐 web `/interrupt` 语义）
- **`crates/tui/src/app_loop_actions.rs`**：`cancel_running_turn` 移除 `clear_pending_inputs` 调用与 `store` 参数——行保留在 store+镜像，下次 submit 的 drain 或 `>` 面板 drain FIFO 消费；刻意不自动重启 drain（用户刚显式取消）。
- **`crates/tui/src/app_helpers.rs`**：删除 `clear_pending_inputs`（唯一调用点消失）及其测试。

### Error 事件重同步镜像
- **`crates/tui/src/app_loop.rs`**：Error 臂改为从 store 重建 queue/steer 两镜像（与 `Done` 的权威重建同款），但不武装 `drain_pending`、保持 `running=false`（防错误循环）。

### admitter 对账竞态补行
- **`crates/tui/src/queue_admitter.rs`**：`AdmitReq`/`AdmitDone` 携带 `display`；`reconcile_ok` 新增 `Reinserted` 结果——temp 行已被权威重建覆盖时把真实行补回尾部（FIFO：更早的行重建时已在面板）。

### steer 武装 plan 交接
- **`crates/tui/src/steer_fire.rs`**：admit 成功后 `chat.note_requirement_submitted()`——plan 模式与 submit/queue 路径同样武装 `plan_submitted`（steer 进来的需求也是需求），act 模式自守卫 no-op。

### handoff 降级不丢输入
- **`crates/tui/src/worker.rs`**：`SwitchAndStart` arm 接线 `handoff_run_prompt`——门失败（`plan_input_count==0`）且捕获输入非空 → 输入作为普通 act prompt 提交；有 handoff 或空输入 → 空 prompt。

### handoff 测试种子修复
- **`crates/tui/tests/handoff_provenance_gate.rs` / `plan_act_handoff.rs`**：4 个测试补 seed `sess.plan_snapshot`（phase-bounded 真源，`SessionState::record` 捕获），使测试在 snapshot-only 与 fallback 两种 `plan_handoff::handoff` 实现下均稳；`stale_double_tap_switch_and_start_preserves_context` 断言更新为新降级语义（4 条消息 + 恰 1 次 LLM 调用）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| cancel 保留 pending（store+镜像，FIFO） | `cancel_running_turn_keeps_pending_rows_in_store_and_mirrors` | `crates/tui/src/app_loop_tests/cancel_keep_pending.rs` |
| Error 重同步镜像、不武装 drain | `fold_error_resyncs_mirrors_from_store` | `crates/tui/src/app_loop_tests/mod.rs` |
| Error+cancelled 不清队列 | `fold_error_when_cancelled_preserves_queue_items` | `crates/tui/src/app_loop_tests/mod.rs` |
| admitter 竞态补行（对账/应用两路） | `reconcile_ok_reinserts_after_done_overwrite_race` / `apply_done_reinserts_after_done_overwrite_race` | `crates/tui/src/app_loop_tests/cancel_keep_pending.rs` |
| steer 武装 plan 交接（plan/act 两态） | `keyboard_steer_in_plan_mode_arms_plan_submitted` / `keyboard_steer_in_act_mode_does_not_arm_plan_submitted` | `crates/tui/src/steer_fire.rs` |
| handoff 降级提交捕获输入（不吞） | `stale_double_tap_switch_and_start_preserves_context` | `crates/tui/tests/handoff_provenance_gate.rs` |
| handoff 成功路径仍折叠（种子后回归） | `plan_phase_input_still_hands_off` + `plan_act_handoff.rs` ×3 | 同上两文件 |
| `handoff_run_prompt` 三分支 | `handoff_run_prompt_only_runs_extra_without_handoff` | `crates/tui/src/worker/tests.rs` |
| `reconcile_ok` 新签名（display 参数） | `queue_admit_offloop.rs` 既有用例更新 | `crates/tui/tests/queue_admit_offloop.rs` |

- 全量回归：`cargo test --workspace`（隔离 worktree = HEAD + 本批变更）→ __TESTS__
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → __CLIPPY__
- 行数：均 ≤800（`queue_admitter.rs` 783 / `steer_fire.rs` 669 / `worker.rs` 638 / `app_loop_tests/mod.rs` 793；新文件 `cancel_keep_pending.rs` 277 ≤400）

## Impact Surface

- TUI 用户：双击 Esc 或 worker Error 后，排队与 steer 输入不再丢失——面板保留、下次 submit/`>` 面板 drain FIFO 消费；plan 模式 Enter-steer 的需求会武装 Shift+Tab 交接；Shift+Tab 门失败时输入框文本以普通 act prompt 提交而非静默丢弃。
- 不影响：web `/interrupt` 行为、store schema、CLI 协议、session 侧 handoff 折叠逻辑（测试种子仅为稳健性，不改 src）。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [shift-tab-double-tap-fake-handoff](../2026-08-18/shift-tab-double-tap-fake-handoff.md)
- [input-delivery-recorded-recovery](../2026-08-18/input-delivery-recorded-recovery.md)
