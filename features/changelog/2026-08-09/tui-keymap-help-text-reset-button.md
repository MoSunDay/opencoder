# feat(tui): 快捷键面板「恢复默认」按钮 + 确认对话框 + Help 文案修正

## 背景

快捷键设置面板（Ctrl+H）存在三处问题：

1. **Help 文案与实际不符**：Help 覆盖层把模式切换主键写成 `Shift+Tab`，且 `Ctrl+T` 与
   `Ctrl+Shift+Tab` 的「清空/保留上下文」语义标注颠倒。此外 Help 缺失 `Ctrl+G`（复制选择模式）、
   `Ctrl+O`（notepad 焦点切换）、`Ctrl+Shift+T`（折叠输入框）三条已实现快捷键。
2. **无「恢复默认」入口**：面板底部仅有「退出 / 帮助」两个按钮，用户一旦误改绑定只能逐条手动改回，
   缺少一键恢复全部默认值的能力。
3. **方向键 bug**：按钮区按 `Left` 应往左退一位（退出←恢复默认←帮助），原实现把 `Left` 也当作
   `+1` 前进，导致反向导航失效。

## 变更

- **`crates/tui/src/keymap_menu/help.rs`**
  - 重写模式切换说明：`Ctrl+T`（主键，保留上下文）、`Alt+Tab`（清空上下文，Shift+Tab 同效）、
    `Ctrl+Shift+Tab`（保留上下文，作为 Ctrl+T 被终端拦截时的后备）。
  - 补入三条缺失快捷键：`Ctrl+G`、`Ctrl+O`、`Ctrl+Shift+T`。
  - 纯静态文案，行为契约不变。
- **`crates/tui/src/keymap_menu/state.rs`**
  - `BUTTON_COUNT` 常量 `2 → 3`；按钮语义改为 `0=退出 / 1=恢复默认 / 2=帮助`。
  - 新增 `confirm_reset: bool` 字段 + `pub fn confirm_reset_open()` getter（供渲染层查询）。
  - 新增 `pub fn reset_to_defaults()`：遍历全部条目，将 spec 改写为 `KeymapConfig::default()`
    对应值（dirty 随之变为 true，退出时走正常 Save patch 流程）。
  - 修复方向键：`Right` → `(+1) % BUTTON_COUNT` 前进，`Left` → `(BUTTON_COUNT - 1)` 反向，wrap 正确。
  - 确认对话框键处理（`confirm_reset` 为真时优先消费）：`Enter`/`y`/`Y` 确认并执行
    `reset_to_defaults()`；`Esc`/`n`/`N` 关闭不重置；其余按键一律 `Idle` 拦截（不泄漏到列表导航）。
  - 按钮 `1` 的 `Enter` 打开确认框（`confirm_reset = true`），不直接重置。
- **`crates/tui/src/keymap_menu/view.rs`**
  - 按钮栏新增「< 恢复默认 >」，选中态高亮逻辑与现有按钮一致。
  - 新增 `render_confirm_reset_overlay()`：顶层居中弹窗，Clear 清底 + 标题/提示行；在
    `confirm_reset_open()` 时绘制于 Help 覆盖层之上。
- **`crates/tui/src/keymap_menu/mod.rs`**
  - 模块文档更新为「退出 / 恢复默认 / 帮助」三按钮 + 21 条目说明。
- **`crates/core/src/config/keymap.rs`**
  - 新增 `hide_composer`（`Ctrl+Shift+T`，折叠/展开底部输入框）绑定：`KEYMAP_INFO` 条目 +
    `KeymapConfig` 字段/默认值/get/set/计数断言（20→21）。
  - 面板第 21 条目的数据来源——菜单动态读取 `KEYMAP_INFO`，必须与菜单变更同提交，
    否则 `new_menu_has_21_entries` / `navigate_*_wraps` 计数失配。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Left 在按钮区反向回退（wrap 至末尾） | `prev_button_goes_backward` | state.rs |
| 按钮 1 + Enter 打开确认框（不立即重置） | `reset_button_opens_confirm` | state.rs |
| 确认框 Enter 执行重置 → entries 复原、dirty=true | `confirm_enter_resets_defaults` | state.rs |
| 确认框 `y` 同样触发重置 | `confirm_y_also_resets` | state.rs |
| 确认框 Esc 取消 → 关闭、entries 保留改动、dirty 不变 | `confirm_esc_cancels` | state.rs |
| 确认框 `n` 同样取消 | `confirm_n_also_cancels` | state.rs |
| 确认框拦截其他键（Down 等）→ 保持打开、Idle | `confirm_intercepts_other_keys` | state.rs |

> 另将 `new_menu_has_20_entries` 更新为 `new_menu_has_21_entries`（条目计数随上游 keymap 增条同步）；
> `reset_to_defaults_restores_original`（既有，验证 reset_to_defaults 行为）保持绿。

- 全量回归（隔离提交 = HEAD + 本提交 7 文件）：`cargo test --workspace` → **2147 passed / 0 failed / 0 ignored**
- keymap_menu 单元测试：`cargo test -p opencoder-tui --lib keymap_menu` → **37 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：help.rs 189；mod.rs 14；state.rs 772（≤ 800）；view.rs 141；core/keymap.rs 196（均合规）

## Impact Surface
- 新增公开符号 `confirm_reset_open()` 仅被 `view.rs` 调用；`render_keymap_popup` /
  `handle_keymap_key` 签名未变，外部调用方（app.rs / app_loop.rs / render.rs）无感。
- `BUTTON_COUNT` 为私有常量；`reset_to_defaults` / `confirm_reset` 为模块内部状态，不跨 crate。
- 重置走既有 `build_patch()` → `Save(patch)` 落盘路径，与手动改绑定的保存流程一致，无新 I/O 形状。
- 纯 TUI 渲染/状态逻辑，不触及 session runner / store 数据形状 / prompt 契约 / 跨进程恢复。
