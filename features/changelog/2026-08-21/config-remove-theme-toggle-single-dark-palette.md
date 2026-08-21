# 去掉 `/config` 主题切换：收敛为单一 Dark 调色板

## 背景

`/config` 表单含 `theme:` 字段（←/→/Space 在 dark/light 间循环）并写盘到
config 的 `theme` 键；TUI 侧由 `ThemeKind` + `OnceLock<RwLock>` 全局态 +
`palette(kind)` 双调色板支撑运行时切换。实际使用只保留一套配色即可——按需求
删掉 `/config` 里的主题修改，只保留当前（dark，即既有默认）一套主题。

## 变更

### TUI

- **`crates/tui/src/theme.rs`**（654 → 514 行）：删除 `ThemeKind`（label/
  from_label/next）、`static THEME`、`set_theme`/`current_theme`、`palette(kind)`
  及 Light 调色板；改为单一 `pub const DARK: Palette`，全部语义色 helper
  （`accent/text/muted/subtle/warn_color/ok_color/err_color/info_color/
  local_color/pink/compaction_color/user_color`）直接取 `DARK` 槽位，
  `highlight_bg()` 固定 `Indexed(238)`。模块 doc 同步改写。
- **`crates/tui/src/model_menu/`**：
  - `config_form.rs`：`ConfigField::Theme` 变体与 `ORDER`（11→10）、
    `ConfigForm.theme` 字段、new() 初始化、build_patch 输出、Left/Right/Space
    三个按键臂全部删除。
  - `patch.rs`：`ConfigPatch.theme` 字段与 `"theme"` JSON 键删除。
  - `view.rs`：`theme:` 行删除。
- **调用点清理**：`app_bootstrap.rs`（2 处启动 set_theme）、`app_loop_model.rs`
  （保存后 set_theme）、`app_loop_envs.rs`（envs 刷新 set_theme + 2 处 doc 提及）
  删除；`ap_menu/view.rs`、`render_tests/{status_ctx,chips,status_bar}.rs`、
  `chat_tests/{plan_card,tool_collapse}.rs` 中测试前置 `set_theme(Dark)` 删除。

### core

- **`crates/core/src/config.rs`**：`pub theme: String` 字段（含 serde
  default_theme）与 `default_theme()`、Default 初始化删除。全仓无
  `deny_unknown_fields`，旧 config.json 里残留的 `"theme"` 键反序列化时被忽略，
  无迁移需求。
- **`crates/core/src/config/merge.rs`**：`has_editable_key`/`merge_into` 的
  theme 分支删除（merge 不再认识该键，残留键按既有未知键语义忽略）。
- 测试夹具中 `"theme":"light"/"dark"` 全部换为 `fps`/`model` 等仍存在的键
  （`config/tests.rs`、`config/domain.rs`、`tests/config_envs_contract.rs`、
  `tests/domain_config_files.rs`；`domain_config_files.rs` 用 `prov/model`
  而非单字符 id——`Config::save` 拒绝 <2 字符 model id）。
- 仓库自身 `.opencoder/config.json` 的 `"theme": "dark"` 移除。

### 文档

- `features/index.md`：删除「dark/light 主题切换」能力条目（历史 changelog
  链接保留）。
- `agents/tui/index.md`：theme 模块描述改为单一 `const DARK: Palette`；
  `/envs` 刷新清单去掉 theme。

## 语义要点

1. 只剩一套主题 = 原 dark 默认配色，所有渲染颜色不变（`DARK` 值与原 dark
   分支逐槽一致，`dark_palette_matches_constants` 守护）。
2. `/config` 焦点链：reasoning → interleave → max_tokens → context_size →
   threshold → fps → ap_max_iter → tmux → Save → Cancel（10 项）。
3. 旧 config.json 中的 `"theme"` 键：加载/合并均忽略，不报错不重写（下次
   `Config::save` 全量写盘时自然消失）。

## 测试覆盖（rules/01 / 02 / 03）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| patch JSON 不再含 theme 键 | `config_patch_serializes_all_fields`（改断言 `v.get("theme").is_none()`） | `crates/tui/src/model_menu/tests/config_tests.rs` |
| Enter 焦点链（10 项，无 Theme） | `enter_chains_through_config_fields_to_save` | 同上 |
| DARK 调色板与 const 语义色一致 | `dark_palette_matches_constants` | `crates/tui/src/theme.rs` |
| merge 未知键硬剪裁仍成立（fps 替换 theme 夹具） | `merge_into_hard_cuts_domain_keys_from_config_json` / `legacy_domain_keys_table` | `crates/core/src/config/merge.rs` |
| envs 重载刷新链路（夹具去 theme） | `envs_outcome_tests`（`recapture_active_env_refreshes` 等） | `crates/tui/src/app_loop_tests/envs_outcome_tests.rs` |
| 域分层配置文件读写（fps/model 替换 theme 夹具） | `config_envs_contract` / `domain_config_files` 全套 | `crates/core/tests/` |

- 回归：`cargo test --workspace` 全绿（3203 passed / 0 failed），
  `cargo clippy --workspace --all-targets` 0 warning，`cargo fmt --check` 干净。
- 行数：theme.rs 514（迭代文件 ≤ 800），无新增文件。

## 影响 / 边界

- 不影响：`/model`、`/envs` 其余刷新链路（client 重建、labels、fps ticker）、
  context_meter 阈值红黄绿、CLI/Web/session/store 边界。
- 用户可见变化：`/config` 少一行 theme；曾把 theme 设为 light 的用户恢复为
  唯一 dark 配色。
