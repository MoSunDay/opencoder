Commit: (working-tree, post-33b5ba2)

# /envs 环境配置管理：独立环境层插入配置解析链

每个环境（env）是 `~/.opencoder/envs/<name>/` 下的**完整配置快照**（`config.json` + 三域文件 `mcp.json`/`cli.json`/`skills.json`，0o600 owner-only），激活后在配置解析链中插入一层：**project > env > ~/.opencoder > XDG**。TUI `/envs` 菜单、web REST `/api/envs`、CLI 经 `Config::load` 自动受益。

## 用户可见变更
- **TUI `/envs`**（别名 `/env`）：两页弹窗——List 页（↑↓ 选择、Enter 激活/取消激活、`c` 新建、`r` 重新捕获、`d` 删除（y/N 确认）、Esc 取消）+ Form 页（输入名字、实时合法性校验、可选从当前配置捕获、Enter 创建、Esc 返回）。激活/取消/删除/重捕获后自动重建 client（`resolve_endpoint`）、刷新 model_label/theme，并广播 `UiCmd::ReloadConfig`。
- **Web REST**：`GET /api/envs`（列表 + active）、`POST /api/envs {name, capture_current=true}`（400 非法名 / 409 重名）、`PATCH /api/envs {active: name|null}`（404 未知环境）、`POST /api/envs/:name/recapture`、`DELETE /api/envs/:name`；所有会改变有效配置的变更向全部 live session 扇出 `DrainCmd::ReloadConfig`（与 `PATCH /api/config` 同机制）。
- **CLI**：`config show` 在 stdout JSON 之前向 stderr 打一行 `active env: <name>`（stdout 保持纯 JSON 可机器消费）。

## 核心语义
- **激活标记**：`~/.opencoder/envs/active` 纯文本文件（单行环境名）。目录被删除的 stale 标记读取时静默视为未激活（fallback 基础链）。
- **解析插入**：`config_candidates_with(workdir, active)` 在项目候选与全局候选之间插入 `~/.opencoder/envs/<name>/config.json`；三域文件同理获得 env 候选（`effective_path_with`：项目 > env > 全局，整体遮蔽不变）。
- **写目标截断**：激活期间 `save_target` 候选截断为 3 个（2 项目 + env）——`/model`、`/config` 等交互式编辑落进 env 而非全局；取消激活后恢复原行为。新建（候选全无）时落 env 层 `config.json`（激活期）或项目根 `opencoder.json`（未激活），均不落全局。
- **捕获（capture）语义**：以 `active=None` 的基础链快照——不含 env-var overlay；config.json 快照剥离三域键；三域文件按域逐一快照；env 内 stale 域文件（基础链已无对应文件）删除。**WYSIWYG：含项目层**（捕获的就是当前会话看到的有效配置）。
- **删除顺序**：先清 active 标记再删目录（激活态删除不会留下 stale 标记）。
- env 文件含 API key 可能，一律 0o600 owner-only 权限写入。

## 变更文件
- core：新 `src/config/envs.rs`（294 行：`active_env`/`set_active_env`/`list_envs`/`create_env`/`recapture_env`/`delete_env`/`validate_env_name`/`envs_home`/`env_dir`、`capture_into`、`write_private_json`）；`config/env.rs` 增 `config_candidates_with`；`config/domain.rs`（443）增 `env_domain_path`/`effective_path_with`/`write_target_with`/`read_effective_with`（删除死代码 `effective_path` 包装）；`config.rs`（783）声明 + re-export envs、`save_target` env 感知；`lib.rs` re-export。
- tui：新 `src/envs_menu/{mod,state,list,form,view}.rs`（EnvsMenu::{List,Form}、EnvsOutcome 七态、BASE_ROW=0 显式无选中）；新 `src/app_loop_envs.rs`（234，`handle_envs_outcome` + `refresh_after_env_change` 镜像 /model 的重建流程）；接线 lib.rs/command.rs（`/envs`+`/env` 别名）/app_loop*.rs/app.rs（799）/frame.rs/render.rs（798）。
- web：新 `src/api_envs.rs`（181）+ lib.rs 路由注册；新 `tests/web_envs.rs`（316）。
- cli：`src/session_cmd.rs` 增 `active_env_banner()` + `config show` stderr 行。
- 修复真实 bug：`selected_env()` 原用 `saturating_sub` 导致 row 0 误映射为空名，现 BASE_ROW=0 显式返回 None。

## 功能 → 测试名
| 功能 | 测试 |
| --- | --- |
| env 层优先级（project > env > global） | `core tests/config_envs_contract.rs::env_layer_sits_between_project_and_global` |
| 三域文件 project > env > global 遮蔽 | `core tests/config_envs_contract.rs::domain_files_shadow_project_env_global` |
| stale 标记静默回退基础链 | `core tests/config_envs_contract.rs::stale_marker_falls_back_to_base`、`core src/config/envs.rs::tests::marker_roundtrip_and_stale_fallback` |
| 捕获 = 基础链快照（无 env-var overlay / 不含 active env） | `core tests/config_envs_contract.rs::capture_snapshots_base_chain_without_env_overlay` |
| 重捕获替换 stale 域文件 | `core tests/config_envs_contract.rs::recapture_replaces_stale_env_files` |
| 激活期写路由落 env、去激活还原 | `core tests/config_envs_contract.rs::save_routes_to_env_while_active_and_back_after_deactivation` |
| 无可编辑候选时在 env 内新建 | `core tests/config_envs_contract.rs::save_creates_env_config_when_nothing_editable_exists` |
| 激活期项目层仍优先于 env | `core tests/config_envs_contract.rs::project_files_still_win_save_target_while_env_active` |
| 删除 active 先清标记 | `core tests/config_envs_contract.rs::delete_active_env_clears_marker_and_restores_base`、`core src/config/envs.rs::tests::create_rejects_duplicates_and_delete_clears_marker_first` |
| env 文件 0o600 owner-only | `core tests/config_envs_contract.rs::env_files_are_owner_only_on_unix` |
| 环境名合法性 | `core src/config/envs.rs::tests::validate_env_name_accepts_and_rejects` |
| `/envs` + `/env` 别名解析与分发 | `tui command::tests::{parse_envs_full_and_alias, dispatch_envs}` |
| List 页激活/去激活与导航 | `tui envs_menu::list::tests::{enter_activates_env_and_base_deactivates, navigation_clamps_between_base_and_last_env, esc_cancels}` |
| List 页操作键门控（无选中时 e/d 空转、n 开 Form） | `tui envs_menu::list::tests::{e_and_d_require_an_env_row, n_opens_form_with_existing_names}` |
| Form 实时校验（非法/重名阻断提交） | `tui envs_menu::form::tests::{enter_submits_only_a_valid_non_duplicate_name, invalid_and_duplicate_names_block_submit}` |
| Form 输入（捕获开关/空格/粘贴/Esc） | `tui envs_menu::form::tests::{capture_toggle_and_space_typing, esc_returns_to_list_and_paste_edits_name}` |
| 激活重建 client 并通知 worker | `tui app_loop_tests/envs_outcome_tests.rs::activate_env_refreshes_config_and_notifies_worker` |
| 去激活还原基础配置 | `tui app_loop_tests/envs_outcome_tests.rs::deactivate_via_base_row_restores_base_config` |
| Form 创建捕获并回 List | `tui app_loop_tests/envs_outcome_tests.rs::create_from_form_captures_and_reopens_list` |
| 删除后刷新 | `tui app_loop_tests/envs_outcome_tests.rs::delete_active_env_clears_marker_and_refreshes` |
| 重捕获刷新 | `tui app_loop_tests/envs_outcome_tests.rs::recapture_active_env_refreshes` |
| 纯导航零重载 | `tui app_loop_tests/envs_outcome_tests.rs::navigation_is_idle_no_reload` |
| REST 列表 + active | `web tests/web_envs.rs::list_reports_envs_and_active` |
| REST 创建 409/400 | `web tests/web_envs.rs::create_rejects_duplicate_and_bad_name` |
| REST 捕获落盘 | `web tests/web_envs.rs::create_with_capture_seeds_env_from_base_chain` |
| REST 激活/去激活 + 404 | `web tests/web_envs.rs::patch_activates_and_deactivates` |
| REST 重捕获 + 404 | `web tests/web_envs.rs::recapture_updates_env_files_from_current_base` |
| REST 删除清标记 + 404 | `web tests/web_envs.rs::delete_removes_env_and_clears_active_marker` |
| ReloadConfig 扇出到 live handle | `web tests/web_envs.rs::activation_fans_reload_config_to_live_handles` |
| CLI config show banner | `cli session_cmd::tests::active_env_banner_tracks_active_env` |

## 回归 gate
`cargo clippy --workspace --all-targets -- -D warnings` ✓ / `cargo build --workspace` ✓ / `cargo test --workspace` 全绿（2822 passed / 0 failed）。行数红线：新增文件全部 ≤400（最大 316），迭代文件 ≤800（app.rs 799 / render.rs 798 / config.rs 783）。
