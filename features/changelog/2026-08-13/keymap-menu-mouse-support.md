Commit: (working-tree, pre-initial-commit)

# Ctrl+H KeymapMenu 底部按钮栏鼠标点击支持

## 背景
KeymapMenu（Ctrl+H 调出）底部有一条三按钮栏：**退出** / **恢复默认** / **帮助**。此前这些按钮只能通过键盘操作（Tab 聚焦到按钮区 → 数字键/方向键选择 → Enter 激活），鼠标点击完全无效。

本次为按钮栏增加鼠标左键点击支持。采用三阶段管线（render → register Rect → hit-test → reuse keyboard action）：渲染阶段记录每个按钮的屏幕空间 `Rect`，鼠标事件到达时做命中测试，命中后复用键盘路径已抽取的 `activate_button` 激活逻辑——保证鼠标与键盘产生完全一致的 `KeymapOutcome`，副作用应用代码零分叉。

## 变更

### 新增模块：鼠标命中分发
- **`crates/tui/src/keymap_menu/mouse.rs`**（163 行，新增）：导出 `pub(crate) fn handle_keymap_mouse(menu, btn_rects, col, row, kind) -> KeymapOutcome`（`mouse.rs:19`）。逻辑顺序：① `confirm_reset_open()` 或 `help_open()` 时返回 `Idle`（overlay 打开期间强制走键盘）；② 仅响应 `MouseEventKind::Down(MouseButton::Left)`，滚动/拖拽/右键一律 `Idle`；③ 遍历 `btn_rects` 用 `render::in_rect` 命中测试，命中则 `select_button_for_click(idx)` 设焦点后调用 `activate_button`。无菜单或空 `btn_rects` 直接 `Idle`。内联 10 条单元测试。

### 模块声明
- **`crates/tui/src/keymap_menu/mod.rs`**：新增 `pub mod mouse;` 声明。

### 状态层：激活逻辑抽取共享
- **`crates/tui/src/keymap_menu/state.rs`**（794 行）：
  - 新增 `pub(crate) fn activate_button(menu, idx) -> KeymapOutcome`（`state.rs:220`）——从键盘 Enter 分支抽取的共享激活逻辑：idx 0=退出/Quit、1=恢复默认/打开 confirm、2=帮助/overlay。
  - 新增 `pub fn select_button_for_click(&mut self, idx)`（`state.rs:116`）——设置 `selected_button` 并把焦点切到 `Buttons`。
  - 将 `handle_keymap_key` 的 Enter 分支重构为直接调用 `activate_button`，消除键盘与鼠标两路径的重复代码。

### 渲染层：按钮 Rect 注册
- **`crates/tui/src/keymap_menu/view.rs`**（162 行）：`render_keymap_popup` 签名扩展，新增 `btn_rects: &mut Vec<Rect>`（`view.rs:14-18`）；新增 `register_button_rects(out, popup, rows)`（`view.rs:123`）——按弹出窗几何 + CJK 宽度感知计算三个按钮的屏幕空间 `Rect` 并回填。
- **`crates/tui/src/render.rs`**（765 行）：`MouseHits` 结构体新增 `pub keymap_btns: Vec<Rect>` 字段（`render.rs:58`）；在 notepad 分支（`render.rs:151`）与主渲染路径（`render.rs:225`）两处清空；`render_keymap_popup` 调用透传 `&mut hits.keymap_btns`（`render.rs:325`）。

### 事件路由：鼠标门控 + 副作用复用
- **`crates/tui/src/app_loop.rs`**（688 行）：抽取 `pub(crate) async fn apply_keymap_outcome(outcome, config, keymap, workdir, cmd_tx)`（`app_loop.rs:647`）共享副作用应用器，供键盘/鼠标两路径调用；新增 `pub(crate) async fn handle_keymap_mouse_event()`（`app_loop.rs:675`）——调用 `handle_keymap_mouse` 后委托 `apply_keymap_outcome`。
- **`crates/tui/src/app.rs`**（800 行）：`Event::Mouse` 分支顶部新增 2 行门控（`app.rs:713-715`）——当 keymap 菜单打开时委托给 `handle_keymap_mouse_event`，命中 Quit 则 break 主循环，否则跳过 chat-body 的 `handle_mouse`。

### 测试辅助同步
- **`crates/tui/src/app_helpers_tests/mouse_tests.rs`**（377 行）与 **`mouse_helpers.rs`**（140 行）：`MouseHits` 结构体初始化处补 `keymap_btns: Vec::new()`，消除字段缺失编译错误。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 点击「退出」按钮 → Quit | `click_exit_button_quits` | keymap_menu/mouse.rs |
| 点击「恢复默认」→ confirm 对话框 | `click_reset_button_opens_confirm` | keymap_menu/mouse.rs |
| 点击「帮助」→ help overlay | `click_help_button_opens_overlay` | keymap_menu/mouse.rs |
| 点击按钮区空白处 → Idle | `click_blank_area_is_idle` | keymap_menu/mouse.rs |
| confirm-reset 打开时忽略鼠标 | `mouse_ignored_when_confirm_reset_open` | keymap_menu/mouse.rs |
| help overlay 打开时忽略鼠标 | `mouse_ignored_when_help_open` | keymap_menu/mouse.rs |
| 滚轮事件 → Idle | `scroll_up_is_idle` | keymap_menu/mouse.rs |
| 右键 → Idle | `right_click_is_idle` | keymap_menu/mouse.rs |
| 鼠标拖拽 → Idle | `mouse_drag_is_idle` | keymap_menu/mouse.rs |
| 空 btn_rects → Idle | `empty_rects_returns_idle` | keymap_menu/mouse.rs |

- TUI 单元测试：1241 → 1251（+10 mouse.rs 新增）
- 全量回归：`cargo test --workspace` → 2446 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：app.rs 800 ≤ 800；state.rs 794 ≤ 800；render.rs 765 ≤ 800；app_loop.rs 688 ≤ 800；mouse.rs 163 ≤ 400；view.rs 162 ≤ 400

## Impact Surface
- **用户可见**：Ctrl+H 快捷键菜单的退出/恢复默认/帮助三个按钮现在可直接鼠标左键点击，体验与键盘一致。
- **不影响**：键盘路径语义（Enter 分支重构后行为等价）、Store trait、ChatStream、session 运行时、Web/CLI 结构；overlay 打开期间鼠标被忽略，强制走键盘确认/关闭，无新交互风险。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
