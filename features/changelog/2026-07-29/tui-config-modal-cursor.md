Commit: (working-tree, pre-initial-commit)

# feat(tui): /config 模态框文本字段光标定位 + 输入框光标守卫

## 背景

`/config` 配置模态框（`ModelMenu::Config`）渲染表单时，终端光标始终隐藏——用户
编辑 `max_tokens` / `ctx size` / `threshold` / `fps` / `ap_max_iter` 等数字字段时
看不到光标位置。同时，输入框的 `place_cursor`（`render.rs`）在模态框打开时仍会
执行 `set_cursor_position`，与模态光标逻辑冲突。

## 变更

### 行为

1. **输入框光标守卫**（`crates/tui/src/render.rs`）：`place_cursor` 调用增加
   `model_menu.is_none()` 门控——模态框打开时跳过输入框光标定位，避免覆盖模态光标。
2. **模态文本字段光标定位**（`crates/tui/src/model_menu/view.rs`）：
   `render_config_form` 在 `render_widget` 之后追加光标定位块——仅当聚焦字段为
   文本编辑字段时，经 `text_field_row`（字段→行号映射）+ `focused_raw_input`
   （字段→原始输入缓冲区）计算 `cx = popup.x + 1 + 15 + raw.chars().count()`
   / `cy = popup.y + 1 + row`，调 `f.set_cursor_position` 将光标置于原始数字输入末尾
  （装饰性后缀如 ` tokens (≈128k)` 之前）。非文本字段（开关 / 按钮）不定位，光标
   保持隐藏。

### 辅助函数（`view.rs`，纯函数）

- `text_field_row(field: ConfigField) -> Option<usize>`：文本编辑字段到表单
  `lines` vec 行号的映射（MaxTokens=2 / ContextSize=3 / Threshold=4 / Fps=5 /
  ApMaxIter=10），非文本字段返回 `None`。
- `focused_raw_input(form: &ConfigForm) -> Option<&str>`：聚焦字段对应的原始
  输入缓冲区引用，非文本字段返回 `None`。

改动隔离于渲染层，不触及 hit-rect / 数据形状 / Store / ChatStream。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| max_tokens 文本字段光标定位到原始输入末尾 (24,9) | `config_form_cursor_on_max_tokens` | `model_menu/tests/config_tests.rs` |
| ctx size 文本字段光标定位到原始输入末尾 (26,10) | `config_form_cursor_on_context_size` | `model_menu/tests/config_tests.rs` |
| 非文本字段（Reasoning 开关）不定位光标，保持 (0,0) | `config_form_cursor_hidden_on_toggle` | `model_menu/tests/config_tests.rs` |
| 非文本字段（Save 按钮）不定位光标，保持 (0,0) | `config_form_cursor_hidden_on_save_button` | `model_menu/tests/config_tests.rs` |

全部为集成测试（`#[cfg(test)]` 模块内），使用 ratatui `TestBackend` + `Terminal` +
`render_model_popup`，经 `assert_cursor_position` 断言精确坐标。无 LLM / DB / 网络。

### 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test --workspace` | PASS — **1307 passed; 0 failed; 0 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build --workspace` | PASS — Finished |

防修绿扫描：无 `#[ignore]`、无删测试、无弱断言、无调试输出、无密钥。

## Impact Surface

- `/config` 模态框文本字段聚焦时显示终端光标（位于数字末尾），非文本字段光标隐藏。
- 模态框打开时输入框不再定位光标，避免与模态光标冲突。
- 不影响：drain 语义 / Store / ChatStream / runner / web / cli。改动隔离于 TUI 渲染层。

## 风险与回退

- 低风险：`render.rs` 守卫仅增加一个 `is_none()` 条件；`view.rs` 变更纯增量
  （`render_widget` 后追加光标块 + 两个无副作用纯函数）。
- 回退：删除 `render_config_form` 中光标定位块 + 两个辅助函数 + `render.rs` 中的
  `model_menu.is_none()` 条件即可。

## 行数

- `crates/tui/src/model_menu/view.rs`：552 行（< 800 迭代中上限）
- `crates/tui/src/render.rs`：759 行（< 800）
- `crates/tui/src/model_menu/tests/config_tests.rs`：505 行（< 800）
