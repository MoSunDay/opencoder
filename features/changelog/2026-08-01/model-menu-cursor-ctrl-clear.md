Commit: (working-tree, pre-initial-commit)

# feat(tui): /config、/model 表单光标跟随 + Ctrl+L/Ctrl+U 清空当前字段

## 背景

- `/config`（ConfigForm）已有光标但 Ctrl 全部被 `state.rs` 吞掉；`/model` 的
  ProviderForm 完全无光标，数字/文本字段编辑时无位置反馈。
- Ctrl+L / Ctrl+U 在主输入框语义不变（Ctrl+L 折叠/清空输入、Ctrl+F 强制刷屏），
  新清空语义仅在 `/config` `/model` 弹窗内生效，两处互不干扰。

## 变更

### Ctrl 分发（`crates/tui/src/model_menu/state.rs`）

- 仍全局 Ctrl+D（含 `\u{4}` 原始形态）→ Quit；其余 Ctrl 仍吞掉，
  **仅放行** `Char('l'|\u{c})` 与 `Char('u'|\u{15})` 给表单处理器
  （沿用 Ctrl+D 双形态匹配惯例，兼容 kitty keyboard protocol 原始控制字符）。

### 表单清空语义（`config_form.rs` / `provider_form.rs` / `headers.rs`）

- `/config`：Ctrl+L/U → 清空聚焦的数字字段（max_tokens / context_size /
  threshold / fps / ap_max_iter）；toggle（Reasoning 等）与按钮字段 no-op。
- `/model` ProviderForm：Ctrl+L/U → 清空 name（仅非只读）/ model_id / base_url /
  api_key_input；**api_key 同时置 `api_key_edited = true`**（对齐 Backspace 分支，
  保证清空被持久化）。headers 子模式优先路由到 `headers.handle_key`。
- HeadersEditor：Ctrl 特判一处放行 Ctrl+L/U → 清空 `active_string()`（当前 name/value）。

### 光标（`crates/tui/src/model_menu/view.rs`）

- `render_provider_form` 仿照 `render_config_form` 的既有公式加光标：
  Name（仅非只读）/ ModelId / BaseUrl / ApiKey 行 `cx = popup.x+1+15+raw.len()`、
  `cy = popup.y+1+row(0..3)`；ApiKey 用原始缓冲（未编辑时为空 → 光标在字段起点）。
- headers 子模式下 `cx = popup.x+1+(5 或 28)+len`、`cy = popup.y+1+5+i`
  （name 列起点 5，value 列起点 28 = 5 空格 + 20 宽 name + " = "）。
- Save/Cancel 按钮、只读 name 不显光标。

### 帮助文案（`crates/tui/src/keybind.rs`）

- 新增一行说明弹窗内 Ctrl+L / Ctrl+U 清空当前聚焦字段。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+U 清空聚焦数字字段且焦点不动 | `ctrl_u_clears_focused_numeric_field` | `crates/tui/src/model_menu/tests/config_tests.rs`（unit） |
| Ctrl+L 与原始 `\u{c}`/`\u{15}` 形态均生效 | `ctrl_l_clears_focused_field_and_raw_control_char_forms_match` | 同上 |
| toggle/按钮字段为 no-op | `ctrl_clear_is_noop_on_toggle_and_button_fields` | 同上 |
| Ctrl+D 在 /config 仍 Quit | `ctrl_d_still_quits_in_config` | 同上 |
| Ctrl 清空 model_id/base_url | `ctrl_clear_empties_model_id_and_base_url` | `provider_tests.rs`（unit） |
| 可编辑 name 清空、只读 name no-op | `ctrl_clear_editable_name_but_readonly_name_is_noop` | 同上 |
| api_key 清空后 `api_key_edited == true` | `ctrl_clear_api_key_marks_edited` | 同上 |
| Ctrl+D 在 /model 表单仍 Quit | `ctrl_d_still_quits_in_provider_form` | 同上 |
| provider 表单光标（ModelId 行） | `provider_form_cursor_on_model_id` | 同上（render 断言） |
| provider 表单光标（ApiKey 用原始缓冲） | `provider_form_cursor_on_api_key_uses_raw_buffer` | 同上 |
| Save 按钮 / 只读 name 不显光标 | `provider_form_cursor_hidden_on_save_and_readonly_name` | 同上 |
| headers 激活行光标（name / value 单元格） | `provider_form_cursor_inside_headers_cell`、`provider_form_cursor_inside_headers_value` | 同上 |
| headers 子编辑 Ctrl+U/L 清空 name/value（含原始控制字符） | `ctrl_u_clears_active_name`、`ctrl_l_clears_active_value`、`ctrl_clear_raw_control_char_forms_match` | `crates/tui/src/model_menu/headers.rs`（unit） |
| 主输入框 Ctrl+L 语义不被弹窗改动影响 | `ctrl_t_not_intercepted_ctrl_l_clears_ctrl_f_redraws`（既有，回归） | `app_helpers_tests/mod.rs` |

> 全部 unit/render 层，零 I/O / DB / 网络依赖。光标断言仿照既有
> `config_tests.rs` 的 `assert_cursor_position` 模式。

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test -p opencoder-tui --lib model_menu` | PASS — 85 passed / 0 failed（落地时点） |
| `cargo check --workspace`（非 test） | PASS — Finished |
| `cargo test --workspace` 全量 | PASS — 1519 passed / 0 failed（含并行 agent 合入后的全量） |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — Finished, 0 warnings |
| `cargo build --workspace` | PASS — Finished |
