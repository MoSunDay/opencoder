Commit: (working-tree, post-33b5ba2)

# autopilot 三态模式（off / ap / review）+ `/ap` 模式选择菜单

autopilot 从布尔开关升级为三态模式 `config.autopilot.mode: off | ap | review`（默认 off）；TUI `/ap` 从「一键翻转」改为打开三选一模式菜单（选择即保存）。

## 用户可见变更
- **`/ap` 交互变更**：原先直接翻转 `autopilot.enabled`，现在弹出模式菜单（克隆 `/skill` 菜单的垂直切片）：`off · 关闭` / `ap · 完全自动` / `review · 自动 review`，↑↓ 移动、Enter 选择即保存、Esc 取消；初始高亮当前模式。
- **新 `review` 模式**：初始任务跑完后自动切 plan agent + 激活 review skill，做**一次性** review turn（合成 review prompt + 单次 run_loop），完成后清 skill 并发 `Done`；不进 ACT/VERIFY 循环。事件面新增 `AutoPilot { phase: Review, iteration: 1 }`（CLI/TUI 走既有 `{phase:?}` 渲染，web SSE 透传）。
- **`ap` 模式**：与旧 `enabled=true` 行为完全一致（PLAN→ACT→VERIFY 自驱循环）。
- **chip**：ap → `AP`，review → `RV`（新增），off → 无。
- **旧配置迁移（防静默关闭）**：config 合并层 `mode` 键优先；无 `mode` 但有旧 `enabled` → `true` 映射 `ap` / `false` 映射 `off`；serde 宽松反序列化，残留 `enabled` 键忽略。web `PATCH /api/config` 自动支持 `mode`。
- `/config`（`/model` 表单）保留 `max_iter` 编辑不动；模式入口收敛到 `/ap` 菜单（避免双入口打架）。

## 变更文件
- core：`config/autopilot.rs`（`ApMode` 枚举 + merge 迁移）、`config/merge.rs`（editable keys + `mode`）、`config.rs`/`lib.rs` 导出、新契约测试 `tests/config_autopilot_contract.rs`（原 config_contract.rs 已 838 行超 800 红线）、`tests/config_contract.rs` autopilot 条目改 mode。
- session：`runner/mod.rs::run` 尾部三态分发；新文件 `autopilot/review_pass.rs`；`autopilot/phases.rs` `switch_agent`/`activate_review_skill` 提为 `pub(super)` 复用；`autopilot/prompts.rs` 新增 `review_prompt`；`autopilot/state.rs` `ApPhase::Review` 变体；测试 `tests/autopilot.rs` helper 改 `..Default::default()` + 新文件 `tests/autopilot_review.rs`。
- tui：新 `ap_menu/`（list/state/view）+ `app_loop_ap.rs`；接线 `app.rs`/`app_loop_actions.rs`/`app_loop.rs`/`frame.rs`/`render.rs`/`command.rs`；`local_cmd.rs` 删除 `toggle_ap`/`ap_marker_text` 及测试（收缩 291→145 行，保留 `/ps` `/stop`）。
- todos：`parent.rs`/`execution.rs` 强制关闭点 `enabled=false` → `mode=Off`（防自驱动外泄）。
- e2e：`scripts/e2e/cli_scenarios.py` E18 与 `web_scenarios.py` E18b 的 autopilot 配置改 `{"mode":"ap"}`、断言改 `mode==ap`。

## 测试清单（功能 → 测试名）
- 三态枚举解析/迁移：`core config::autopilot::tests::{mode_parses_all_three_states_and_ignores_unknown, legacy_enabled_maps_ap_and_off, mode_wins_over_legacy_enabled_when_both_present}`
- mode 落盘 round-trip：`core tests/config_autopilot_contract.rs::mode_roundtrips_all_three_states`
- 旧 `enabled` 迁移不静默关：`core tests/config_autopilot_contract.rs::legacy_enabled_migrates_instead_of_silently_disabling`
- 深合并保留：`core tests/config_autopilot_contract.rs::mode_survives_partial_deep_merge`；`config_contract.rs::autopilot_config_roundtrips_through_save`
- 默认 off 守卫：`core tests/config_autopilot_contract.rs::default_mode_is_off`
- Off 零事件：`session tests/autopilot.rs::autopilot_off_mode_emits_no_autopilot_events`
- Review 一次性触发（恰好一条 Review 事件 + 无 Plan/Act/Verify）：`session tests/autopilot_review.rs::review_mode_runs_exactly_one_review_pass`
- Review skill 激活后清理：`session tests/autopilot_review.rs::review_mode_activates_then_clears_review_skill`
- Ap 回归（max_iterations=1 仍整轮）：`session tests/autopilot_review.rs::ap_mode_with_max_iterations_one_still_cycles_phases`
- 菜单 patch JSON 形状：`tui ap_menu::list::tests::ap_mode_json_shape_has_only_the_mode_key`、`mode_index_covers_every_mode`
- 菜单状态机（高亮/移动/Enter 保存/Esc 取消/空槽）：`tui ap_menu::state::tests::{new_highlights_current_mode, up_down_move_with_wrap, enter_saves_selected_mode_json_and_closes, esc_and_ctrl_d_cancel_and_close, empty_slot_is_idle, unmapped_key_is_idle_and_keeps_menu}`
- 弹窗渲染：`tui ap_menu::view::tests::popup_renders_title_choices_and_current_mark`
- `/ap` 接线打开菜单：`tui app_loop_slash_action_tests::slash_action_ap_parses_and_opens_mode_menu`
- 三态 chip AP/RV：`tui render_tests::chips.rs::ap_chip_reflects_autopilot_mode`
- local_cmd 收缩回归：`tui local_cmd::tests::{is_local_matches, is_local_non_matches, run_unknown_falls_through}` 等（toggle_ap 系列删除）

## 回归 gate
`cargo clippy --workspace --all-targets -- -D warnings` ✓ / `cargo build --workspace` ✓ / `cargo test --workspace` 全绿（exit 0）。
