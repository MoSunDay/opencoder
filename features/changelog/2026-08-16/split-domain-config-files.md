Commit: 4027d70

# 配置按域拆分：mcp.json / cli.json / skills.json 独立域文件（硬切，breaking）

## 背景

- `config.json` 单文件膨胀：`mcp_servers` / `cli` / `skills` 三个 map 形状的注册表键与模型/Provider 等通用标量配置混在一个文件里，条目增删频繁、体积不成比例。
- 三域键与通用配置生命周期不同：TUI 各菜单各管一域（`/mcp`、`/cli`、`/skill`），却共用同一个 config.json 写目标。
- 处理方式为**硬切**（无兼容读）：遗留 `config.json` 中的三域键加载时直接忽略，迁移责任显式交给用户。

## 变更

- 新增 `crates/core/src/config/domain.rs`（400 行）：域键表 `DOMAIN_FILES`（`mcp_servers`→`mcp.json`、`cli`→`cli.json`、`skills`→`skills.json`）及查找/写目标/读/存/分流的纯函数集（`effective_path` / `write_target` / `read_effective` / `save_domain` / `split_patch` / `apply_domain`）。
- **查找**：恰两个候选——项目 `<workdir>/.opencoder/<domain>.json` 优先，其次全局 `~/.opencoder/<domain>.json`（与 `~/.opencoder/config.json` 同一 home，经 `env::global_opencode_home`，`scoped_config_home` 覆盖同样生效）。项目文件存在则**整体遮蔽**全局文件（单一生效文件，不跨文件逐键合并，区别于 config.json 的全候选深合并）。**XDG 目录不参与域文件查找**（config.json 候选链查 `~/.config/opencoder/`，域文件不查）。
- **写目标**（`write_target`）：已有项目文件 → 已有全局文件 → 双无时保存**新建全局**文件。
- **保存**（`save_domain`）：JSON merge-patch 写入（pretty-print，父目录自动创建）；`null` 条目删除该键；已有但损坏（不可解析且非空白）的目标文件**拒绝写入**（返回 Err，文件字节不动）。加载侧（`read_effective`）损坏或合法但非对象的域文件 warn 后视为不存在（坏域文件不阻断启动）。
- `Config::load`（`crates/core/src/config.rs`）：config.json 候选链合并完成后，按 `DOMAIN_FILES` 顺序加载三域文件（`apply_domain` 逐条目合并，缺省字段保留兄弟）。
- `Config::save`（同文件）：`split_patch` 分流——域键写各自域文件，其余键走原 `save_target` + `save_to` 的 config.json 流程。返回路径语义：混合 patch → 返回 config.json 路径（域写入仍执行）；仅域键 patch → 返回最后一个域写入目标，**不创建 config.json**；空 patch（无域键）→ 维持原 config.json-only 行为。
- `Config::merged_with`（同文件）：patch 中的域键仍经 `apply_domain` 应用（从 JSON patch 构建 Config 的测试 helper 语义保留）。
- **硬切**（`crates/core/src/config/merge.rs`）：`merge_into` 不再消费 config.json 中的三域键（遗留键忽略、不报错）；`has_editable_key` 不再把三域键视为可编辑键——只含三域键的 config.json 不再成为保存目标。三域条目合并循环（含 mcp `env` 的 `{VAR}` 间接引用解析）自 `merge_into` 移入 `domain.rs::apply_domain`，语义不变。

## 兼容性

- **breaking**：遗留 `config.json` 仍含 `mcp_servers` / `cli` / `skills` 的，三键加载时被**忽略**（其余键不受影响）。迁移：把三个键的对象**原样**（内部 JSON 形状不变、值不变）移入对应域文件——`mcp_servers` → `mcp.json`、`cli` → `cli.json`、`skills` → `skills.json`（项目 `<workdir>/.opencoder/` 或全局 `~/.opencoder/` 均可）注意域文件根即条目 map：如 `mcp.json` 直接是 `{"<server>": {...}}`，**不带** `mcp_servers` 包裹键（即取 config.json 中该键的**值**作为域文件根）。
- 两个显式设计决策：XDG 目录不参与域文件查找（与 config.json 候选链不同）；项目与全局域文件双无时写**全局**。
- 项目域文件整体遮蔽全局（不做跨文件逐键合并）；损坏域文件拒写、加载视为不存在；`null` 条目删键；`Config::save` 返回路径语义见上。
- TUI `/mcp`、`/cli`、`/skill` 菜单与 Web 配置写接口的 patch 形状不变（仍是 JSON merge-patch），仅落点分流到域文件。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 域键表映射（键→文件名、非域键排除） | `domain_key_table_maps_keys_to_files` | `crates/core/src/config/domain.rs` |
| 项目级域文件路径构造 | `project_domain_path_lives_under_opencoder_dir` | `crates/core/src/config/domain.rs` |
| 全局域文件路径随 scoped home 重定向 | `global_domain_path_honors_scoped_home` | `crates/core/src/config/domain.rs` |
| split_patch 原样分离域键（含 null 删除值） | `split_patch_separates_domain_keys_verbatim` | `crates/core/src/config/domain.rs` |
| split_patch 空 patch 产出空拆分 | `split_patch_empty_patch_yields_empty_split` | `crates/core/src/config/domain.rs` |
| apply_domain 条目路由到对应 Config 字段 | `apply_domain_routes_entries_to_the_right_field` | `crates/core/src/config/domain.rs` |
| apply_domain 条目合并保留兄弟字段/开关 | `apply_domain_entry_merge_preserves_siblings_and_toggles` | `crates/core/src/config/domain.rs` |
| apply_domain 忽略非对象值与未知键 | `apply_domain_ignores_non_object_values_and_unknown_keys` | `crates/core/src/config/domain.rs` |
| mcp env `{VAR}` 间接引用解析（迁自 merge.rs） | `apply_domain_resolves_mcp_env_indirection` | `crates/core/src/config/domain.rs` |
| 硬切：config.json 三域键被忽略 | `merge_into_hard_cuts_domain_keys_from_config_json` | `crates/core/src/config/merge.rs` |
| 双无 → 保存新建全局域文件 | `save_creates_global_domain_file_when_neither_exists` | `crates/core/tests/domain_config_files.rs` |
| 项目域文件整体遮蔽全局 | `project_domain_file_shadows_global_entirely` | `crates/core/tests/domain_config_files.rs` |
| null 条目删除域键 | `null_patch_entry_deletes_domain_key` | `crates/core/tests/domain_config_files.rs` |
| 损坏域文件拒写 + 加载视为不存在 | `corrupt_domain_file_refuses_write_and_loads_as_absent` | `crates/core/tests/domain_config_files.rs` |
| 非对象域文件加载视为不存在 | `non_object_domain_file_loads_as_absent` | `crates/core/tests/domain_config_files.rs` |
| 遗留 config.json 三域键加载时忽略 | `legacy_config_json_domain_keys_are_ignored_on_load` | `crates/core/tests/domain_config_files.rs` |
| 混合 patch 分流（余键写 config.json） | `mixed_patch_routes_domain_and_config_keys_separately` | `crates/core/tests/domain_config_files.rs` |
| 仅域键 patch 返回域目标、不建 config.json | `domain_only_patch_returns_domain_target_and_skips_config_json` | `crates/core/tests/domain_config_files.rs` |
| 空 patch 维持 config.json-only 行为 | `empty_patch_still_writes_config_json` | `crates/core/tests/domain_config_files.rs` |
| load 从三个域文件读取条目 | `load_reads_entries_from_all_three_domain_files` | `crates/core/tests/domain_config_files.rs` |
| TUI /mcp 菜单 save/toggle/delete 落 mcp.json、null 删键、不建 config.json | `handle_mcp_outcome_save_toggle_delete_write_mcp_domain_file` | `crates/tui/src/app_loop_tests/mcp_outcome_tests.rs` |
| TUI 损坏 mcp.json 拒写并推 save-failed 标记 | `handle_mcp_outcome_save_failure_pushes_error_marker` | `crates/tui/src/app_loop_tests/mcp_outcome_tests.rs` |
| TUI /cli 菜单 save/toggle/delete 落 cli.json | `handle_cli_outcome_save_toggle_delete_write_cli_domain_file` | `crates/tui/src/app_loop_tests/cli_outcome_tests.rs` |
| TUI /skill 开关落 skills.json、不建 config.json | `skill_toggle_writes_skills_domain_file_not_config_json` | `crates/tui/src/app_loop_tests/skill_outcome_tests.rs` |
| Web PATCH /api/config 域键写 skills.json 且 GET 反映 | `patch_config_writes_skills_domain_file` | `crates/web/tests/web_api_ops.rs` |

## 回归

- `cargo build --workspace` → PASS（Finished，零错误）
- `cargo clippy --workspace --all-targets -- -D warnings` → PASS（零警告，exit 0）
- `cargo test --workspace` → PASS（2694 passed / 0 failed / 0 ignored；基线 f6ae527 为 2670，净增 24、无删除）
