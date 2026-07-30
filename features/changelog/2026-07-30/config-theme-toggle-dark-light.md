# feat(tui): /config 主题切换（dark/light，默认深色）

## 背景

此前 `theme.rs` 是一组静态 `pub const Color`（深色基调）+ 零状态辅助函数（见
[tui-theme-modernization](./tui-theme-modernization.md)）。颜色在编译期固定，
运行时无法切换；`Config` 也没有主题字段。用户无法选择浅色主题以适配白底终端。

目标：给 `/config` 表单加一个主题切换按钮，默认深色（与现状一致），可循环切到浅色，
选择持久化到 opencoder.json 并热生效（无需重启）。

## 变更

### Config 持久化主题字段
- **`crates/core/src/config.rs:77`** — `Config` 新增 `pub theme: String`，`#[serde(default = "default_theme")]`；`:117` `default_theme() -> "dark"`；`Default` 设 `theme: default_theme()`。
- **`crates/core/src/config/merge.rs:18`** — `has_editable_key` 识别顶层 `"theme"` 键（使 `/config` 写回路径不被判定为空）。
- **`crates/core/src/config/merge.rs:137`** — `merge_into` 应用 `"theme"` 字符串到 `cfg.theme`。

### 运行时主题系统（theme.rs：静态 const → 动态 palette）
- **`crates/tui/src/theme.rs`** — 保留全部 9 个 `pub const`（dark 默认，向后兼容），新增：
  - `ThemeKind { Dark, Light }`（`:40`）+ `label()/from_label()/next()`。
  - `Palette` 数据 + **纯函数** `palette(kind) -> Palette`（`:87`）：dark（与 const 一致）/ light（白底可读：text=Black、accent=Blue、muted=Gray、subtle=DarkGray、warn=LightRed、err=Red、local=Magenta）。
  - 全局状态 `static THEME: OnceLock<RwLock<ThemeKind>>` + `set_theme()`（`:117`）/ `current_theme()`（`:125`，默认 Dark）。纯 std，无新依赖。
  - 语义颜色函数（运行时按 `current_theme()` 解析）：`accent()/text()/muted()/subtle()/warn_color()/ok_color()/err_color()/info_color()/local_color()`（`:137+`）+ `highlight_bg()`（`:149`，dark=Indexed(238)/light=Indexed(252)）。
  - **重接辅助函数内部为动态语义色**（dark 下行为不变）：`rounded_block_plain`→`muted()`、`rounded_block_focus`→`accent()`、`list_highlight`→`highlight_bg()`、`context_meter`→`err/warn/ok_color()`、`agent_chip_fg`→`warn/accent()`、`muted_style/subtle_style/local_style`→对应语义色。

### /config 表单接入
- **`crates/tui/src/model_menu/config_form.rs:87`** — `ConfigField::Theme`；`:105` `ORDER` 加入（位于 `ApMaxIter` 与 `Save` 之间，长度 14）。`ConfigForm.theme: ThemeKind`，`new()` 经 `from_label(&config.theme)` 初始化。`handle_key` 在 `←/→/Space` 上 `form.theme = form.theme.next()`；`build_patch` 输出 `theme.label()`。
- **`crates/tui/src/model_menu/patch.rs`** — `ConfigPatch.theme: String`；`to_json` 顶层 `"theme"`。
- **`crates/tui/src/model_menu/view.rs`** — `render_config_form` 在 `ap_max_iter` 行后、`[Save]` 前插 `theme:` 行；`focus_style/dim_style/val_style` 改用 `warn_color()/subtle()/text()`，`field_line` hint 用 `muted()`（dark 下完全不变）。`want_h`/`text_field_row` 索引**不变**（新增行填入既有余量），现有光标测试不受影响。

### 启动 / 热重载应用主题
- **`crates/tui/src/app_bootstrap.rs:29`** — `Config::load` 后立即 `set_theme(from_label(&config.theme))`，启动即对齐配置。
- **`crates/tui/src/app_loop.rs:400`** — `/config` Save 成功 reload 后 `set_theme(from_label(&reloaded.theme))`，先于 `ReloadConfig` 重绘，切换即时生效。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Config 默认 theme=dark | `theme_defaults_to_dark` | `crates/core/src/config.rs` |
| merge 识别 theme 键 | `has_editable_key_recognizes_theme` | `crates/core/src/config.rs` |
| merge_into 应用 theme | `merge_into_applies_theme` | `crates/core/src/config.rs` |
| ThemeKind label 往返/大小写 | `theme_kind_label_roundtrip` | `crates/tui/src/theme.rs` |
| ThemeKind next 循环 | `theme_kind_next` | `crates/tui/src/theme.rs` |
| dark palette 对照常量 | `palette_dark_matches_constants` | `crates/tui/src/theme.rs` |
| light palette 值 | `palette_light_text_is_black` | `crates/tui/src/theme.rs` |
| 全局 set/get | `set_then_current_theme` | `crates/tui/src/theme.rs` |
| ConfigPatch 序列化 theme | `config_patch_serializes_all_fields` | `model_menu/tests/config_tests.rs` |
| Space 循环主题 Dark→Light | `config_form_theme_cycles_with_space` | `model_menu/tests/config_tests.rs` |
| Enter 链经 Theme 到 Save | `enter_chains_through_config_fields_to_save` | `model_menu/tests/config_tests.rs` |

- 全量回归：`cargo test --workspace` → **1406 passed, 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **零警告**
- 行数：`theme.rs` 394 ≤ 400；其余改动文件均远在 800 内

## Impact Surface
- 用户：`/config` 表单新增 `theme:` 行，`←/→/Space` 在 `dark`/`light` 间循环，Save 持久化到 opencoder.json；默认 `dark`（行为与此前一致）。
- dark 主题下所有颜色**完全不变**（const 保留 + 辅助函数经动态函数解析回相同值）。
- light 主题生效范围：theme.rs 辅助函数覆盖的边框/高亮/仪表/chip/样式，以及 `/config` 表单自身的焦点/数值/hint 着色。直接硬编码 `Color::*` 的模块（chat/render 等）暂不随主题切换——留待后续迁移。
- 不影响：CLI/session/store/web/LLM 边界（仅 Config 新增一个顶层字符串字段）。

## Related Docs
- [agents/tui](../../agents/tui/index.md)（theme 条目已更新为支持运行时切换）
- [既有 tui-theme-modernization](./tui-theme-modernization.md)（本次在其静态集中化基础上扩展为动态切换）
