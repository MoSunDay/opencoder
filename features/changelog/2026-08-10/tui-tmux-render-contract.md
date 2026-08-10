Commit: 63017f50ff7b37620616de9bae586363b8cceff5

# tmux 渲染链路按职责闭环

## Context

Mac iTerm2 经 SSH 进入 tmux 后仍出现闪屏与字符错位。`tmux capture-pane` 显示 pane 内部的正文、滚动条、输入框和弹窗边界全部对齐，证明应用到 tmux 的逻辑网格正确；偏差发生在 tmux 向外层客户端输出的下游链路。应用逐帧完整重绘、DECSET 2026 和逐 cell 绝对定位无法控制 tmux 的客户端差分器，反而把 10 FPS 动画放大为高带宽整屏更新。

同时，当前 iTerm2 profile 未启用 Unicode 9+ widths，而 ratatui/tmux 使用较新的 Unicode 宽度表。应用、tmux 与终端对同一字符的 cell width 不一致时，Unicode 内容本身无损也仍会发生视觉错位。

## Change Summary

- 恢复标准 `CrosstermBackend` 与 ratatui 增量 diff，tmux 与直接终端使用同一渲染路径。
- 移除应用级 DECSET 2026、tmux 逐帧完整重绘和逐 cell 绝对定位 backend。
- 保留启动、resume、真实 resize 以及用户显式 force-redraw 的单次 clear；稳态渲染绝不 clear。
- 保留背景色空格滚动条，不重新引入 ambiguous 方块字符。
- 明确 iTerm2 profile 契约：`Ambiguous characters are double-width` 关闭，`Use Unicode version 9+ widths` 开启。

## Impact Surface

- tmux 中动画与流式输出恢复为变化单元格级别的输出量，消除整屏刷新放大的闪屏。
- 中文、emoji 和圆角边框继续原样输出，不通过 ASCII 降级规避问题。
- 无新增配置、环境变量、数据库字段或公开接口。

## Validation

- frame 单测保留 tick 边界与 `u32` wraparound 行为。
- render stale-cell 回归覆盖双缓冲轮换后缩短内容会被空格擦除。
- TUI 测试与 workspace 编译用于确认标准 backend 路径完整闭环。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
