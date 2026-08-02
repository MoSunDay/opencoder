Commit: (working-tree, pre-initial-commit)

# feat(tui): /ap 开关命令 + 右上角 AP 指示 + /config 移除 autopilot 开关

## 背景

autopilot 的开关此前只存在于 TUI `/config` 表单（`ConfigField::ApEnabled`），
表单保存时会把 `autopilot.enabled` 一并写回。运行时缺少直观指示：右上角 chip
区域没有任何 AP 显示。本次把开关从表单挪到 `/ps` `/stop` 同类的纯显示命令
`/ap`（不污染 context），并在 composer 右上角用品红 `AP` chip 常驻指示开启态。

## 变更

### 右上角 AP chip（`crates/tui/src/render.rs`、`frame.rs`、`app.rs`）

- `render()` / `render_frame()` 末尾新增 `ap_enabled: bool` 参数；
- autopilot 开启时在 composer 右上角渲染品红（`theme::local_color()`，与 `/ps`
  `/stop` marker 同色系）的 `AP` chip；瞬态 mode-flash / copy-status chip 绘制在
  后，短暂覆盖 AP 属预期。

### /config 移除 autopilot 开关（`model_menu/config_form.rs`、`patch.rs`、`view.rs`）

- 删除 `ConfigField::ApEnabled`（枚举、ORDER、字段、init、Left/Right/Space 三个
  toggle 分支）；`ap max_iter` 数值项完整保留。
- `ConfigPatch` 删除 `ap_enabled`；`to_json()` 的 `"autopilot"` 只写
  `{"max_iterations": ...}`——merge 深合并保证磁盘上既有 `enabled` 不被覆盖
  （core 契约测试 `config_contract.rs` 已验证）。
- `view.rs` 删除 `autopilot:` 开关行，保留 `ap max_iter:` 行。

### `/ap` 开关命令（`command.rs`、`local_cmd.rs`、`app_loop.rs`、`app.rs`）

- `COMMANDS` 新增 `("/ap", "切换 autopilot 自动模式（不计入模型上下文）")`；
  `SlashAction::Ap` 接入 popup 与自由输入两条路径。
- `local_cmd::run` 改为 async，新增 `config: &mut Config` / `cmd_tx` / `workdir`
  参数；`is_local` 纳入 `"ap"`（自由输入路径因此自动跳过 push_user /
  context_used / start_turn，不污染 context）。
- `toggle_ap`：翻转 `autopilot.enabled` → `Config::save` + `Config::load` →
  `*config = reloaded` → `UiCmd::ReloadConfig`（复用 `/config` 保存链路）→ 品红
  marker `[ap] autopilot: on|off`；保存/重载失败推红 marker（仿 `[/config]` 报错
  样式）。

### worker 顺带修正（`worker.rs`）

- `ReloadConfig` 仅在 model 字符串实际变化时才 persist store 列并广播
  `SessionEvent::ModelSwitch`——`/ap` 或纯 max_iter 保存不再冒出无关的
  `[model]` marker。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| /ap popup Enter 分发 | `enter_on_ap_dispatches` | crates/tui/src/command.rs |
| /ap parse（popup + 自由输入） | `parse_local_commands` | 同上 |
| is_local 纳入 /ap | `is_local_matches` / `is_local_non_matches` | crates/tui/src/local_cmd.rs |
| marker 文本 on/off | `ap_marker_text_reflects_target_state` | 同上 |
| 翻转持久化 on→off→on（tempdir 真 save/load） | `toggle_ap_round_trips_persisted_state` | 同上 |
| 非 local 命令穿透 | `run_unknown_falls_through` | 同上 |
| AP chip 门控（开启品红可见 / 关闭缺席） | `ap_chip_visible_only_when_autopilot_enabled` | crates/tui/src/render_tests/chips.rs |
| 表单 Enter 链不再含 ApEnabled | `enter_chains_through_config_fields_to_save` | crates/tui/src/model_menu/tests/config_tests.rs |
| patch 不再写 enabled、保留 max_iterations | `config_patch_serializes_all_fields` / `config_patch_omits_max_tokens_when_none` | 同上 |
| 表单初始化仅取 max_iter | `config_form_inits_autopilot_from_config` | 同上 |
| 同 model ReloadConfig 不冒 [model] | `reload_config_same_model_emits_no_model_switch` | crates/tui/src/worker/tests_reload.rs |

- 全量回归：`cargo test --workspace` → **1592 passed; 0 failed**（102 个测试二进制，
  当次实跑，2026-08-01）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **零警告**（当次实跑）
- build：`cargo build --workspace` → **零错误**（Finished，当次实跑）

## Impact Surface

- **可感知影响**：TUI 内 `/ap` 翻转 autopilot（popup + 自由输入两路径均可），
  开启时右上角品红 `AP` chip 常驻；`/config` 表单不再有 autopilot 开关（`ap
  max_iter` 保留）；`/ap`/纯 max_iter 保存不再冒 `[model]` marker。
- **不影响**：web / CLI headless（autopilot 配置行不变）、session runner 循环、
  消息/事件存储形状、`/ps` `/stop` 行为。
- **兼容**：磁盘配置的 `autopilot.enabled` 仍被深合并保留，老 `/config` 保存的
  值不丢。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [features/index.md](../../../features/index.md)
- [autopilot-loop changelog](../2026-07-28/autopilot-loop.md)
