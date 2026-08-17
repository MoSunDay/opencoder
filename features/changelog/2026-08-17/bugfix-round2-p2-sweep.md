# Bug 扫除第二轮：P2 快赢（6 项）

范围延续第一轮红线：不碰 skill 语义区与并行 WIP 文件（question_menu 为并行会话在途工作，本轮未触碰）。

## 修复清单

11. **help 页宣传已摘除的 Ctrl+Shift+T**（`tui/keymap_menu/help.rs`）：删除该行；防回归断言按行首 token 精确匹配（朴素 `!contains` 会被合法的 `Ctrl+Shift+Tab` 前缀误伤），并断言 `Ctrl+G` 邻行仍在。
12. **整域 null patch 把 domain 文件写成字面 `null`**（`core/config/domain.rs::save_domain`）：merge 后归一化非对象 root 为 `{}`——整域删空落盘 `{}` 而非 `null`，且修复"null 文件再 save 丢域内内容"的连锁问题。选 `{}` 而非删文件：保持文件存在与 corrupt-refusal 语义。
13. **env 名 `active` 与激活 marker 冲突**（`core/config/envs.rs`）：`validate_env_name` 拒绝精确 `"active"`（marker 大小写敏感，仅小写精确值）；`list_envs` 过滤存量 `envs/active/` 目录（历史遗留容错）。create/set/delete/recapture 经 validate 自然一致。
14. **MCP 服务器名归一化冲突**（`tui/mcp_menu/patch.rs`、`tui/app_loop_mcp.rs`、`session/mcp/pool.rs`）：`a-b`/`a.b`/`a_b` 归一化同前缀 `a_b` → 工具覆盖/作用域越权。三层防护：`normalized_server_name`+`colliding_server` 纯函数（TUI 侧真源）；`/mcp` 保存时校验，命中冲突不落盘并以红色 marker 提示改名（改名让位与原位更新不算冲突）；`tools_for` 经新纯函数 `merge_tools` 按 server 名排序合并，full_name 冲突 `tracing::warn` 并丢弃重复（先注册者胜、跨运行确定）。存量已冲突配置不强制迁移。未触碰 `llm_call.rs`。
15. **InjectionTarget 静默吞未知 tag 与空数组**（`core/config/cli.rs`）：`apply_tag` 返回是否识别；未知 tag / 空数组 `tracing::warn`（不硬错，保前向兼容），反序列化语义不变。
16. **todos 四项**（`todos/src/{batch,parent,runner,domain,transitions}.rs`）：
    - 非 Running 结果丢弃不炸整轮：apply_result Ok 路径先查状态，迟到结果（外部中断/同批 rewind 后）log 后丢弃；
    - 空/非法决策纠错重问（限次）：`decide` JSON 解析失败同 session 纠错重问（共 3 次）；`validate_dispatch` 纯校验提取（dispatch 行为不变），drive_inner 的 Dispatch 分支 dry-run 失败带 CORRECTION 重问 schedule（重问 2 次），仍失败按原样 bail；
    - `resume` 拒绝 status==Running：防双驱动，错误信息含 `opencoder todos interrupt <id>` 接管指引（CLI Interrupt 子命令为接管路径）；
    - `validate_spec` 校验每个 todo 的 agent 可解析、is_primary、非 workflow（从执行期提前到提交期）。

## 测试清单（rules/01）

- #11 `tui keymap_menu::help::tests::help_no_stale_hide_composer_shortcut`
- #12 `core domain::tests::{save_domain_whole_null_empties_existing_file_to_object, save_domain_whole_null_on_empty_dir_writes_empty_object, save_domain_after_whole_null_keeps_new_entries_and_deletions}`
- #13 `core envs::tests::{validate_env_name_rejects_marker_reserved_name, create_env_rejects_active_name_without_touching_fs, list_envs_filters_legacy_active_directory}`
- #14 `tui mcp_menu::patch::tests::{normalized_server_name_is_table_driven, colliding_server_detects_normalized_twin, colliding_server_ignores_vacated_rename_key, colliding_server_ignores_disjoint_names, colliding_server_ignores_same_original_name}`、`tui app_loop_mcp::tests::{patch_server_keys_extracts_added_and_removed, handle_mcp_outcome_refuses_save_colliding_after_normalization, handle_mcp_outcome_allows_non_colliding_save_alongside_similar_name}`、`session mcp::pool::tests::{merge_tools_drops_duplicate_after_normalization, merge_tools_first_registrant_follows_input_order, merge_tools_keeps_disjoint_names}`
- #15 `core cli::tests::{apply_tag_reports_known_and_unknown_tags, apply_tag_legacy_alias_expands_to_explore_and_build, empty_inject_to_array_yields_all_false_target}`
- #16 `todos tests/interrupt.rs::{external_interrupt_after_successful_todo_discards_result_cleanly, rewound_sibling_discards_late_successful_result}`、`tests/recovery.rs::{unparseable_parent_decision_is_corrected_without_suspending, non_runnable_dispatch_is_corrected_without_suspending, three_unparseable_parent_decisions_suspend_with_error_preserved}`、`tests/runtime.rs::resume_rejects_running_workflow_until_interrupted`、新文件 `tests/dispatch_validation.rs`（validate_dispatch 单测 6 条）、`domain.rs::tests`（agent 校验拒绝用例）

## 回归 gate（rules/02）

`cargo test --workspace --no-fail-fast`：**2902 passed / 0 failed** ✓
`cargo clippy --workspace --all-targets -- -D warnings`：零警告 ✓
