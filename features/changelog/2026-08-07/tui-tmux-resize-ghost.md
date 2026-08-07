Commit: 339e7326ea9598c3813782549ae3c34854a379d6

# TUI tmux resize 残影修复

## Context

启动阶段的 `Terminal::clear()` 只能清理进入 TUI 时已经可见的区域。tmux 隐藏状态栏或 pane 尺寸变化后，ratatui 会把新暴露的单元格初始化为空白，不会对 tmux 持久 grid 中的旧字符产生差异输出，因此信息展示区仍可能出现旧数据残影。

## Change Summary

- 显式 `Event::Resize` 路径在 `autoresize()` 后执行 `Terminal::clear()`。
- 帧 ticker 的 idle-size 轮询发现丢失的 resize 时执行相同的同步与清屏。
- 未发生尺寸变化时不清屏，避免常规帧轮询引入额外刷新。
- 尺寸同步和清屏的 I/O 错误上报到 TUI 主循环，不再静默保留部分刷新状态。

## Impact Surface

- tmux 状态栏隐藏后的 TUI 首次 pane 扩展
- tmux pane 放大、缩小及快速拖拽
- crossterm `Resize` 事件丢失后的尺寸轮询补偿

## Validation

- `resize_event_clears_the_physical_grid`
- `idle_resize_clears_the_physical_grid`
- `idle_poll_without_resize_does_not_clear`
- `cargo test -p opencoder-tui --lib`: 956 passed

## Related Docs

- `agents/tui/index.md`
