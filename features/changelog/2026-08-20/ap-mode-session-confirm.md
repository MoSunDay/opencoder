# `/ap` 两步确认：y=存全局默认，n=仅本 session（复刻 `/model` 交互）

## 背景

`/ap` 菜单此前按 Enter **立即写盘**（全局默认），没有"只试一下"的退路——与
`/model` 统一后的两步确认语义（2026-07-26 `model-switch-default-unify`）相悖。
本次照抄 `/model`：选中后弹「存为全局默认？」确认框，`y`/Enter=写全局、
`n`=仅本 session 生效（持久化到 `sessions.autopilot_mode` 列，resume 后仍有效，
同 `sessions.model` 语义）、Esc=取消。session 级生效靠 runner 分发点读
override 实现——后续任何全局 config reload 都不会冲掉 session 设置。

上一会话遗留的 store v11 列 WIP 本次收敛补全（core/session/tui 全链路）。

## 实现

- **store**（schema v11）：`sessions.autopilot_mode TEXT` 列（NULL=跟随全局），
  `SessionMeta`/`SessionPatch` 加 `autopilot_mode` + `clear_autopilot_mode`
  （三态 set 语义同 model），INSERT/SELECT/UPDATE 全接线。
- **core**（`config/autopilot.rs`）：`ApMode::parse(&str)` / `ApMode::as_str()`
  公开——config merge、resume 校验、store 列写入共用一套 wire 拼写。
- **session**：
  - `SessionState.ap_mode_override: Option<ApMode>` + `effective_ap_mode()`
    （override 优先，否则 `config.autopilot.mode`）；runner 尾部分发点改读它
    （净 0 增行）。子代理仍走 config 强制 Off（`SessionState::new` 不继承
    override）。
  - `resume.rs`：从行恢复 override；非法值 warn+忽略（回退全局）。
  - `fork.rs`：1 行继承（同 model，task_type/requirement 等仍重置）。
- **tui**：
  - `ap_menu/state.rs`：`ApOutcome` 改为 `Save(ApMode)` /
    `SaveSessionOnly(ApMode)`；Enter 进 `confirm: Option<ApMode>` 确认子状态
    （克隆 `model_menu` 的 `confirm_save_default`：y/Y/Enter=Save、n/N=
    SaveSessionOnly、Esc=Cancel、其余键重挂保持 Idle）。
  - `ap_menu/view.rs`：确认态标题 ` /ap — SAVE AS DEFAULT? … ` + 居中浮层
    （62×5，克隆 `/model` 的 `render_save_default_confirm`）。
  - `app_loop_ap.rs`：`Save` → `Config::save(ap_mode_json)` + reload + 发
    `UiCmd::ApModeSwitch` + marker `(global default)`（全局落点随 env 域机制
    分流到 `ap.json`，见同日 `envs-autopilot-domain.md`）；`SaveSessionOnly`
    → 仅内存 config 合并 + `ApModeSwitch` + marker `(session)`。均不发
    `ReloadConfig`（mode 不影响 endpoint，无需重建 client）。
  - `worker.rs`：新 `UiCmd::ApModeSwitch(ApMode)`——置 override + 内存 config
    镜像 + `update_session` 写列（y/n 两路都写，resume 语义与 model 一致）。

## 语义要点

1. `sessions.autopilot_mode` 三态：NULL=跟随全局（新 session 默认）；
   `"off"/"ap"/"review"`=钉死本 session（y 或 n 设置后均如此，全局后续变更
   不影响该 session）；非法值 resume 时 warn 忽略。
2. 分发优先级：`ap_mode_override` > `config.autopilot.mode`。
3. 确认框内 ↑/↓ 被拦截（任意非终止键重挂确认态，同 `/model`）。

## 测试覆盖（rules/01 / 03）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ApMode parse/as_str 与 serde 一致 | `parse_and_as_str_round_trip_all_wire_keys` | `crates/core/src/config/autopilot.rs` |
| 列 round-trip（set/patch/clear） | `autopilot_mode_column_round_trips` | `crates/store/tests/store_integration.rs` |
| Some+clear 互斥拒绝 | `field_and_clear_combinations_are_rejected`（扩用例） | `crates/store/tests/session_patch_conflict.rs` |
| v10→v11 迁移（旧行 NULL、patch 生效、版本 pin） | `schema_migration_v10_to_v11_adds_autopilot_mode` | `crates/store/tests/store_migrations.rs` |
| 分发三态读 override | `effective_ap_mode_tests::{none_follows_config_mode, override_wins_over_config}` | `crates/session/src/lib_tests.rs` |
| resume 恢复 override / NULL 跟随全局 / 非法值忽略 | `resume_{restores_ap_mode_override, null_ap_mode_follows_global_config, ignores_unknown_ap_mode_value}` | `crates/session/tests/resume_ap_mode.rs` |
| fork 继承 | `fork_copies_messages_and_resets_meta`（扩断言） | `crates/session/src/fork.rs` |
| 确认框状态机（Enter 挂起 / y / Enter / n / Esc / 重挂） | `enter_arms_confirm_then_y_saves_global` 等 5 例 | `crates/tui/src/ap_menu/state.rs` |
| 浮层渲染 | `confirm_overlay_renders_save_as_default_hints` | `crates/tui/src/ap_menu/view.rs` |
| worker 写列 + override 钉死 + 无 store 不炸 | `ap_mode_switch_{pins_override_and_persists_column, off_overrides_global_ap_config, without_store_skips_persist_silently}` | `crates/tui/src/worker/tests_ap_switch.rs` |
| app-loop：n 不写盘 / y 写全局并发 cmd / Esc 无副作用 | `ap_{session_only_merges_memory_and_skips_disk, global_save_writes_config_and_notifies, esc_from_confirm_cancels_without_effects}` | `crates/tui/src/app_loop_ap_outcome_tests.rs` |

## 回归门（rules/02）

- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告（终验轮实跑，
  并发会话 theme.rs WIP 收敛后退出 0）
- `cargo fmt --check` → 本特性文件干净；残余 3 处 diff 均在并发会话
  tok-cost WIP 文件（event.rs / tok_cost.rs / key_handler.rs），非本变更路径
- `cargo test --workspace --no-fail-fast` → 终验轮全量实跑 3185 通过 / 0 失败
  （201 套件）；此前负载敏感 flake `queued_skill_fires_at_consumption_not_during_kickoff`
  本轮亦绿（同机的并发会话未跟踪 WIP `file_menu` 曾现 1 例同类 flake，
  隔离单跑与本轮全量均绿，非本变更路径）。
- 分 crate 复核全绿：core 289 / store 111 / session 752 /
  tui 1499(lib)+全部集成套件 / web 113。
