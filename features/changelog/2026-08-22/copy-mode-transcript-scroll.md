Commit: 52622d77bf761b10cf72f97ced5100e2c93961f9

# Ctrl+G copy mode 支持正文滚动

## Context

Ctrl+G copy mode 已能把对话正文投影成无边框、无角色标题和代码框线的 clean view，但活跃状态会消费所有键，包括 PageUp/Down。虽然 `render_clean` 已持有基于 `CleanModel` 的滚动几何，用户仍无法改变 `scroll/follow`，只能复制当前屏幕与最新一页，长对话中的较早信息不可达。

## Change Summary

- copy mode 增加纯函数 `scroll_action` 与 `next_scroll`，只把正文导航键转换为滚动状态；其它键继续被吞掉，不会编辑 composer、触发 modal 或执行命令。
- ↑/↓ 逐行浏览；PageUp 向旧内容移动 20 行；Home 跳到开头；PageDown/End 恢复 follow 并回到最新内容。
- 复用既有 body `scroll/follow`，由 `render_clean` 按净化后的 wrapped-row 总数 clamp。被删除的装饰行不进入滚动计数，屏幕位置与可复制文本保持一致。
- copy-mode 状态 chip 与快捷键帮助补充滚动提示。plan/annotation editor 和 notepad 的 copy 投影仍维持各自既有视图，不宣传正文导航键。

## Impact Surface

- 用户可留在 Ctrl+G 模式内翻阅并选择超过一屏的历史对话，无需退出 clean view。
- PageDown/End 回到底部后继续跟随新输出；向上滚动会关闭 follow，避免选择过程中视图被新 token 拉走。
- 终端原生拖拽选择、Ctrl+G/Esc 退出和 Shift+drag 行为不变。

## Validation

- `cargo test -p opencoder-tui --lib copy_mode::tests`：17 passed，覆盖导航状态机与 PageUp 后真实 clean viewport 显示更早行。
- 快捷键帮助测试与 copy-mode 硬件光标测试各 1 passed。
- `cargo clippy -p opencoder-tui --lib -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check` 通过。

仅运行受影响的 TUI 测试，未执行 workspace 全量。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
