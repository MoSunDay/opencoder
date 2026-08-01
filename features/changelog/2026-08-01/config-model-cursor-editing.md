# feat(tui): /config 数字字段与 /model 文本字段光标编辑（←/→ 移动，输入在光标处插入）

## 背景

- `/config` 数字字段（max_tokens / ctx size / ctx threshold / fps / ap max_iter）
  的 **←/→ 原本做 ±1000/±1 数值调整**，与「打字追加、Backspace 弹尾」的文本编辑
  模型割裂，且光标始终渲染在末尾，无法原位修正。
- `/model` ProviderForm（name / model_id / base_url / api_key）此前 **←/→ 无绑定**，
  同样只能末尾追加。
- 本次统一为标准的行内编辑模型：←/→ 移动光标、字符在光标处插入、Backspace 删光标
  前一字符、终端光标跟随编辑位置。toggle 字段（reasoning / interleave / theme /
  capabilities）的 ←/→ 循环行为保持不变。

## 变更

### `crates/tui/src/model_menu/config_form.rs`
- `ConfigForm` 新增 5 个 per-field 光标：`max_tokens_cursor` / `context_size_cursor` /
  `threshold_cursor` / `fps_cursor` / `ap_max_iter_cursor`（`usize`，char index），
  `new()` 中初始化为各自 buffer 末尾（保留「打字即追加」的默认语义）。
- 删除 `adjust_threshold` / `adjust_context_size` / `adjust_fps` / `adjust_ap_max_iter`
  （唯一调用方 `handle_key`）。
- 新增私有 `edit_numeric(|text, cur| ...)`：按 `focus` 分发到数字字段的 (text, cursor)，
  所有编辑（←/→、字符、Backspace、Ctrl+L/U、paste）统一走 composer 纯函数：
  - ←/→：`cur = cur.saturating_sub(1)` / `(cur + 1).min(len)`，**不改值**；
  - 字符数字：`composer::insert_char`，光标 +1（`idx` 先 clamp 到 len）；
  - Backspace：`composer::backspace`（无返回值时原地不动）；
  - Ctrl+L/U：`text.clear()` 且 `cur = 0`；
  - `paste_into`：过滤非数字后 `composer::insert_str` 在光标处插入，光标前进。
- toggle 的 ←/→ 循环（含 theme 原有前向循环）原样保留。

### `crates/tui/src/model_menu/provider_form.rs`
- `ProviderForm` 新增 `name_cursor` / `model_id_cursor` / `base_url_cursor` /
  `api_key_cursor`，两个构造器（`from_existing` / `new_blank`）初始化为各自 buffer 末尾
  （api_key 原始缓冲为空 → 0）。
- 新增私有 `edit_text(|text, cur| ...)`（Name 仅非只读、ModelId、BaseUrl）；
  ApiKey 因首笔编辑需清空原值并置 `api_key_edited`，保留独立分支。
- ←/→ 从「无绑定」变为光标移动；字符 / Backspace / paste 改为在光标处编辑
  （均走 composer 纯函数，Backspace/字符前 clamp idx 到 len）。
- Ctrl+L/U 清空时同步 `cur = 0`。

### `crates/tui/src/model_menu/list.rs`
- `Char('n')` 新建 ProviderForm 字面量补齐 4 个光标字段。

### `crates/tui/src/model_menu/view.rs`
- 新增 `focused_cursor(form)`（/config）与 `provider_focused_cursor(form)`（/model），
  镜像既有 `focused_raw_input` / 文本字段匹配。
- 光标 x 从 `raw.chars().count()`（恒为末尾）改为
  `composer::cursor_column(raw, idx)`（unicode 显示宽度，含 CJK 宽字符）。
- 帮助文案更新：/config 数字字段与 /model 文本字段提示改为
  `←/→ cursor, digits(或 type), Backspace…`；弹窗标题 `/config — ←/→ cursor`、
  `/model — type, ←/→ cursor`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 光标渲染在字段中间（/config） | `config_form_cursor_renders_at_edit_position` | `model_menu/tests/cursor_editing_tests.rs` |
| ←/→ 移光标不改值（/config） | `left_right_moves_numeric_cursor_without_changing_value` | 同上 |
| 光标 0/len 钳制（/config） | `left_right_cursor_clamps_at_zero_and_len` | 同上 |
| 数字在光标处插入（/config） | `typing_digit_inserts_at_cursor` | 同上 |
| Backspace 删光标前字符（/config） | `backspace_deletes_char_before_cursor` | 同上 |
| Ctrl+L/U 清空并复位光标（/config） | `ctrl_clear_resets_cursor_to_zero` | 同上 |
| 光标渲染（/config 末尾，回归） | `config_form_cursor_on_max_tokens` / `config_form_cursor_on_context_size` | `model_menu/tests/config_tests.rs` |
| ←/→ 移光标不改值（/model） | `provider_left_right_moves_cursor_without_changing_value` | `model_menu/tests/cursor_editing_tests.rs` |
| 字符在光标处插入（/model） | `provider_typing_inserts_at_cursor` | 同上 |
| Backspace 删光标前字符（/model） | `provider_backspace_deletes_char_before_cursor` | 同上 |
| 光标渲染在字段中间（/model） | `provider_cursor_renders_at_edit_position` | 同上 |
| 光标渲染（/model 末尾，回归） | `provider_form_cursor_on_model_id` / `provider_form_cursor_on_api_key_uses_raw_buffer` | `model_menu/tests/provider_tests.rs` |
| 既有打字/Backspace/paste 回归 | `typing_digits_sets_fps` / `typing_digits_sets_max_tokens` / `typing_digits_sets_context_size` / `backspace_pops_digit_from_threshold` / `backspace_pops_digit_from_context_size` / `backspace_clears_threshold_to_empty` / `type_digits_replaces_value` / `config_form_paste_*` / `provider_form_paste_*` | `model_menu/tests/{config_tests,provider_tests}.rs` |

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test -p opencoder-tui --lib model_menu` | PASS — 95 passed / 0 failed |
| `cargo test -p opencoder-tui` | PASS — 796 passed / 0 failed |
| `cargo test --workspace` | PASS — 1587 passed / 0 failed / 1 ignored（1 ignored 为既有 `research_smoke_bing_wikipedia`）|
| 行数 | `config_tests.rs` 679、`provider_tests.rs` 796（迭代 ≤800）；`cursor_editing_tests.rs` 273（新 ≤400） |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 0 warnings |
| `cargo build --workspace` | PASS — Finished |
