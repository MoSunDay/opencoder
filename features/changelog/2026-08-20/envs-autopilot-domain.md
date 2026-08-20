# autopilot 升级为第四个 env 域（ap.json）

## 背景

env 域此前覆盖 mcp/cli/skills 三域（`.opencoder/{mcp,cli,skills}.json`），而
autopilot 配置仍寄居在 `config.json` 顶层 `autopilot` 键——切 env 不跟随、capture
不剥离、`/ap` 写盘落在 config.json 而非 env 层。本次把 autopilot 升级为第四个 env
域：域文件 `ap.json`，与三域完全同权。

## 机制（表驱动，`crates/core/src/config/domain.rs`）

- `DOMAIN_FILES` 从 3 元组扩为 4：新增 `("autopilot", "ap.json")`。save 分流、
  capture 剥离+快照、加载读域文件、`merged_with` 链全部由表驱动自动获得，
  无散落特判。
- `apply_domain` 新增 `"autopilot"` arm：与三域的 entry-map 逐项合并不同，
  ap.json **顶层即 `AutoPilotConfig` 本体**（非 entry map），故走 whole-object
  `super::autopilot::merge`。
- `split_patch` 无需改动：`/ap` 的 patch 形状 `{"autopilot":{"mode":..}}` 不变，
  由既有表逻辑自动分流到 ap.json。

## 语义决策

- 旧 `config.json` 顶层 `autopilot` 键**硬切忽略 + warn**（复刻三域先例，不做
  自动迁移）：`merge.rs` 删去 `merge_into` 的 autopilot 分支与
  `has_editable_key` 的 autopilot 可编辑块，`legacy_domain_keys` 扩为
  `["mcp_servers", "cli", "skills", "autopilot"]`，warn 文案提及 ap.json。
- env 激活时 `/ap` 读写落 env 层（`envs/<name>/ap.json`）；deactivate 回退 base
  链（全局 `.opencoder/ap.json`，项目层可遮蔽）。

## 测试清单

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 双无（全局/项目均无 ap.json）时 `/ap` 写全局 | `autopilot_save_creates_global_ap_json_when_neither_exists` | `crates/core/tests/domain_config_files.rs` |
| 项目 `ap.json` 遮蔽全局并成为 save 目标 | `autopilot_project_ap_json_wins_target_and_shadows_global` | 同上 |
| mixed patch 中 autopilot 键分流到 ap.json、不再写 config.json | `autopilot_mixed_patch_splits_from_config_json` | 同上 |
| apply_domain 对 autopilot 做 whole-object merge 且不伤兄弟域 | `apply_domain_autopilot_merges_whole_object_and_preserves_siblings` | `crates/core/src/config/domain.rs` |
| 核心验收：ap 模式跟随 env 激活/切换/deactivate | `autopilot_mode_follows_env_activation_switch_and_deactivation` | `crates/core/tests/config_envs_contract.rs` |
| capture 剥离+快照 ap.json（扩展现有 capture 用例） | `capture_snapshots_base_chain_without_env_overlay` / `recapture_replaces_stale_env_files` | 同上 |
| has_editable_key 忽略域键（含 autopilot） | `has_editable_key_ignores_domain_keys` | `crates/core/src/config/tests.rs` |
| 旧 config.json autopilot 键硬切忽略（扩表） | `legacy_config_json_domain_keys_are_ignored_on_load` | `crates/core/tests/domain_config_files.rs` |
| e2e ap pin 迁至 `.opencoder/ap.json`（旧位置硬切失效） | `client_e2e.rs` ap 相关用例 | `crates/web/tests/client_e2e.rs` |

## 回归门（rules/02）

- `cargo test --workspace` → **3172 passed / 0 failed**（201 个 result 汇总；首跑 3148/0、复跑 3160/0 亦全绿）
- `cargo clippy --workspace --all-targets` → 零警告
- `cargo fmt --check` → 干净

## 附带修复（半同步 WIP 收敛）

工作树混入上一会话的 session 级 `/ap` WIP（`SessionMeta.autopilot_mode`、store
schema v11、`SessionEvent::LlmUsage`、tui `replay_into_chat` 第 5 参），本次一并
收敛至编译/测试/clippy 全绿：store v11 列与 SELECT 列序（autopilot_mode 排最后）、
~50 处 `SessionMeta` 字面量补字段、版本 pin 10→11、39 处 tui 测试 `&workdir`
needless_borrow 由 `cargo clippy --fix` 机械消除。
