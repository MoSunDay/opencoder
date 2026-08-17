Commit: 7e7da28 (并入并行会话的全仓提交；改动归属本计划第一轮)

# Bug 扫除第一轮：10 项 P1 功能级修复（08-14→08-17 新能力）

范围：只修与并行 WIP 零重叠的文件；skill 语义区域（skill_resolve/skill_context/latent/seed/skill_menu、runner/{mod,steer,event,llm_call,subagent}、resume、bash_guard、autopilot、app_loop、menu、core/config.rs）全部移交另一会话，本轮未触碰。

## 修复清单

1. **Enter 运行中入队忽略 submit 返回值**（`tui/app.rs` + `queue_admitter.rs`）：新增 `admit_running` 辅助函数统一封装 submit + 成功时 `note_requirement_submitted`（plan→act 交接门不再漏计数）；"Enter while running" 分支补 `push_history`——异步 admit 失败的 flash "recover text with ↑ history" 不再撒谎。`handle_queue`（Tab）内部收敛到同一函数。
2. **`/cli` `/mcp` 重命名产生重复条目**（`cli_menu/{form,list}.rs`、`mcp_menu/{form,patch}.rs`）：`save_json`/`save_mcp_json` 增加 `renamed_from` 参数，改名且 ≠ 新名时 patch 域对象同时写旧键 `Value::Null`（merge-patch 删键），旧键不再残留（CLI 双份注入 / MCP 双连接消除）；未改名/新建不自删。
3. **Kitty 键盘 Shift Release 打断 copy 模式原生选择**（`tui/terminal.rs` + `app.rs`）：`consume_modifier_or_release` 感知 `copy_mode`，Release 时仅清标志不 `resume_mouse_capture`（决策提为纯函数 `resumes_on_shift_release`）。
4. **notepad/plan_edit 打开时 Ctrl+G 全键死区**（`tui/copy_mode.rs` + `app.rs`）：`handle_key` 增加 `overlay_active` 参数，overlay 活跃时不 toggle、不吞任何键（含 copy_mode 已激活状态），plan_edit/notepad 正常接收按键。
5. **todos 外部中断被当执行失败**（`todos/{batch,runner}.rs`）：`apply_result` Err 分支先复查 store——workflow 已被外部写为 Suspended 时不做本地 execution_failed、不 commit（不耗 attempt、不误标 Failed、不覆盖外部态）；`drive_inner` 本地 cancel 分支同样先采纳外部代际。有意偏差：外部判定用 `status == Suspended`（而非 `|| generation`），因“仅代际冲突”路径被 pin 测试 `generation_conflict_stops_the_run` 约束。
6. **todos resume 复用陈旧 assistant 消息当 candidate**（`todos/execution.rs`）：run 前快照消息水位，candidate 查找改走 `latest_new_assistant`（`.skip(watermark)`），取消/空转后返回 "no final candidate" 而非上一次尝试的结果。
7. **todos rewind 不重置 attempt → 恢复自锁**（`todos/transitions.rs`）：rewind 对里程碑及全部受影响后代重置 `attempt=0`、`candidate=None`、`next_context_mode=None`（保留 session 指针供 Resume），回退后再 dispatch 不再 bail "exhausted max_attempts"。
8. **onboarding 被非凭据失败劫持成死循环**（`tui/{onboarding,app_bootstrap}.rs`）：`build_ready_client` 错误分类为 `StartupFailure::{Credentials,Unbuildable}`——仅凭据/endpoint 失败进向导；proxy env 非法、非 http(s) base_url、非法 header 等 Unbuildable 失败直接进应用，以 `UnbuildableClient`（每 turn 报 "model client unavailable: …"）浮出根因，不再以退出应用为唯一出口。
9. **recapture_env 空 base 链不清陈旧 env config.json**（`core/config/envs.rs`）：`capture_into` 在 merged 为空时 `remove_file`（NotFound 容忍），恢复 full-replace 语义——陈旧 api_key 不再残留生效。
10. **老配置三域键静默丢弃**（`core/config/merge.rs`）：`merge_into` 检测 `mcp_servers`/`cli`/`skills` 域键（新增纯函数 `legacy_domain_keys`）时 `tracing::warn` 迁移指引；语义不动（硬切 pin 测试保持）。

## 测试清单（rules/01）

- #1 `tui queue_admitter::tests::{admit_running_success_notes_requirement, admit_running_failure_skips_requirement_and_rolls_back}`
- #2 `tui cli_menu::list::tests::{save_nulls_old_key_on_rename, save_keeps_entry_when_name_unchanged, save_without_rename_writes_single_key}`、`cli_menu::form::tests::renaming_existing_entry_nulls_old_key`、`mcp_menu::patch::tests::{save_nulls_old_key_on_rename, save_keeps_server_when_name_unchanged, rename_patch_removes_old_key_after_merge}`、`mcp_menu::form::tests::renaming_existing_server_nulls_old_key`
- #3 `tui terminal::tests::{resumes_on_shift_release_gates_capture_restore, consume_modifier_tracks_shift_in_copy_mode_without_capture_fight}`
- #4 `tui copy_mode::tests::{overlay_active_ignores_toggle_key, overlay_inactive_toggles_normally, overlay_active_does_not_swallow_when_copy_mode_active}`
- #5 `todos tests/interrupt.rs::{external_interrupt_window_keeps_max_attempt_todo_unfailed, local_cancel_after_external_write_adopts_external_state}`
- #6 `todos execution.rs::tests::latest_new_assistant_*`（3 条）+ 新文件 `todos/tests/resume_watermark.rs`（2 条集成）
- #7 `todos transitions.rs::tests::rewind_rewinds_attempt_bookkeeping_so_recovery_can_redispatch`
- #8 `tui onboarding::tests::{startup_failure_classifies_invalid_proxy_as_unbuildable, startup_failure_classifies_non_http_base_url_as_unbuildable, unbuildable_client_fails_every_stream_with_reason}`（+ 既有 readiness 测试适配分类断言）
- #9 `core envs::tests::{recapture_removes_stale_config_json_when_base_chain_emptied, recapture_into_env_without_config_json_is_not_an_error}`
- #10 `core merge::tests::legacy_domain_keys_table`（既有 `merge_into_hard_cuts_domain_keys_from_config_json` pin 保持）

## 回归 gate（rules/02）

`cargo clippy --workspace --all-targets -- -D warnings` 零警告 ✓
`cargo test --workspace`：全绿 ✓（期间并行会话 WIP 文件 `tui/notepad/editor.rs` 的新增测试短暂失败，属其在途工作，本轮提交前已由该会话自行修复复绿，非本轮触碰。）

## 备注

- 三项 todos 修复均做了临时还原变异验证（各新测试精确失败于对应缺陷）。
- 本轮子任务触发过一次全仓 `cargo fmt`，工作区因此出现大量 fmt-only 重排（llm/store/web/cli/core 等）；本轮提交只含上述清单文件的改动。
