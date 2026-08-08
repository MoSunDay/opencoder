# refactor(tui): 移除应用级文本选择与 OSC52 剪贴板（改用终端原生选择）

**Date:** 2026-08-08
**Crate:** `opencoder-tui`
**Baseline tests:** 2125 (workspace, pre-change) → **2072 passed**（net −53；54 TUI 测试随模块删除移除，store 侧 net +1）

## 背景

OpenCoder TUI 此前实现了完整的应用级鼠标拖拽文本选择与 OSC52 剪贴板复制：

1. **`selection.rs`**（507 行，18 个内联测试）：以绝对内容行 `[a,b]`（`screen_row + scroll`）追踪选区，滚动时锚定文本不漂移；松开鼠标 `finish_copy` 提取选中逻辑行，经 `copy_osc52`（vendored base64 + `ESC]52;c;` 序列）写入系统剪贴板，并叠 `render_overlay` 反白高亮。
2. **`clip_probe.rs`**（478 行，24 个内联测试）：探测终端类型 / 显示服务器（Wayland/X11/headless）/ SSH / tmux `set-clipboard`，分派 wl-copy/xclip/xsel/pbcopy 本地工具，缓存探测结果。

问题：

- **OSC52 在多终端下不可靠**（VTE/screen 系列），fallback 链复杂（tmux buffer 等），维护成本高且行为不一致。
- **应用级选择与终端原生选择冲突**：鼠标被 crossterm 捕获后，用户无法用终端自身的拖拽选择复制文本。已有的 `consume_modifier_or_release`（`src/terminal.rs`，Shift 按下挂起鼠标捕获 → EnableMouseCapture 取消）提供了更可靠的替代路径——用户按住 Shift 即可使用终端原生文本选择。

因此移除整套应用级选择 + OSC52 剪贴板，全面转向终端原生选择。

## 变更

### 删除的模块

| 文件 | 行数 | 删除测试数 | 职责 |
|------|------|-----------|------|
| `crates/tui/src/selection.rs` | 507 | 18 | 鼠标拖拽选择 + 反白高亮 + OSC52 复制 |
| `crates/tui/src/clip_probe.rs` | 478 | 24 | 终端/显示服务器/tmux 剪贴板探测 + fallback 链 |
| `crates/tui/src/app_helpers_tests/mouse_clip_tests.rs` | 697 | 7 | clip_probe/选择/复制相关单测 |
| `crates/tui/src/app_helpers_tests/mouse_dbl_click_tests.rs` | 114 | 3 | 双击选词测试 |

合计 52 个测试随上述文件删除（4 文件）。

### 签名变更（参数链瘦身）

`render.rs::render()`、`render_body()` 与 `frame.rs::render_frame()`：
- 删除 `selection: Option<crate::selection::SelRange>` 与 `copy_status`（及其派生的 `copy_status_text` helper）参数。
- 新增 `shift_held: bool`：当 Shift 按下时渲染状态提示 chip（`Shift+drag: select`，warn 色），引导用户使用终端原生选择。
- `render_body` 不再调用 `selection::render_overlay`（反白高亮移除）。

`app_helpers.rs::handle_mouse()` / `pre_key_intercept()`：
- 删除 `selection`、`last_click`、`dbl_click`、`copy_msg` 等可变状态参数。
- 删除 `is_within_dbl_click_window` helper 与 `selection::abs_row_at` 调用。
- 鼠标点击只处理按钮命中 / 滚动 / subagent focus，拖拽选择路径整体移除。

`app.rs`：移除 `selection`/`copy_status`/`last_click`/`dbl_click` 循环局部状态；`shift_held`（既有局部 bool）直接传入渲染。

### 测试文件同步

- `mouse_tests.rs` 裁剪 2 个选择/复制用例（7→5）。
- `mouse_helpers.rs` / `mouse_scroll_tests.rs` / `mouse_wheel_tests.rs` / `arrow_click.rs` / `body.rs` / `chips.rs` / `timer.rs` / `render_clear_tests.rs`：移除对 `selection` / `copy_status` 形参的调用与传值，保持其余断言不变。

## 测试说明

移除的测试全部针对已删除的 `selection` / `clip_probe` 模块及其参数链调用点（功能已不存在）。TUI 侧净删除 54 个测试；store 侧 `store_concurrency.rs` 重写后净 +1（6→7）。workspace 净 −53，与 2125→2072 一致。**无任何"改测试以转绿"行为**——删除的测试所属模块已从源码中物理移除。

存活的鼠标测试仍覆盖：按钮命中（工具 / 折叠 / 跳转 / chip）、滚轮与拖动滚动、subagent focus 切换、composer 光标定位、内联图片渲染等非选择路径。

| 关注点 | 覆盖来源 | 层 |
| --- | --- | --- |
| 终端原生选择（Shift 挂起鼠标捕获） | `terminal::tests` + `consume_modifier_or_release` | unit |
| 鼠标按钮命中 / 滚动 / focus | `mouse_tests` / `mouse_scroll_tests` / `mouse_wheel_tests` | unit |
| 渲染主路径（shift_held 提示 chip） | `render::tests` / frame 集成 | unit |

## Gate

- 构建：`cargo build --workspace` → 干净（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 全量回归：`cargo test --workspace --no-fail-fast` → 2072 passed; 0 failed。
- 行数：所有改动文件 ≤800（render.rs 771、chat.rs 789、app.rs 784、app_helpers.rs 692、frame.rs 远低）。

## 影响面

- **用户**：鼠标拖拽选择改由终端原生处理（按住 Shift 临时挂起 OpenCoder 的鼠标捕获）；应用不再写系统剪贴板（OSC52 路径移除）。状态栏在 Shift 按下时提示选择可用。
- **不影响**：session / store / web / 持久化 / 协议；无数据库、配置、环境变量变化。
- 纯展示层与人机交互简化，业务语义零变化。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
