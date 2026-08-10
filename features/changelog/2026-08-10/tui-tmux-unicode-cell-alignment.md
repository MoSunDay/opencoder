Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# tmux Unicode 单元格绝对定位

## Context

Mac IDE/iTerm2 经 SSH 连接 Linux tmux 时，服务端 ratatui/tmux 与外层终端的 Unicode 宽度判断可能不同。Crossterm 原先把相邻更新连续打印，后一格坐标依赖前一字形的实际显示宽度；字体 fallback 把某些边框、方块或 emoji 解释为不同宽度后，滚动条与 `/` 弹窗边缘会稳定错位。重复 Ctrl+F、重开弹窗或逐帧 clear 只能重放同一几何误差，并分别带来无效重绘或闪屏。

## Change Summary

- tmux 渲染路径增加 grid-safe Crossterm backend，每个 ratatui 单元格先发送绝对坐标，再原样输出 Unicode 符号与样式。
- 非 tmux 路径保持 Crossterm 相邻单元格优化，不增加直接 Linux 终端的输出量。
- 主体与 queue 滚动条改用背景色填充的单空格单元格，并共享同一套比例/端点算法；不再依赖 `┊`、`█` 的字体宽度。
- 保留同步更新、tmux 完整逻辑重绘以及启动/resume/真实 resize 的生命周期 clear；稳态帧不执行物理 clear。

## Impact Surface

- Mac IDE/iTerm2 → SSH → Linux tmux 中的中文、emoji、圆角边框、滚动条和 Slash 弹窗。
- 不增加配置、环境变量、数据库字段或公开接口。
- tmux 完整帧会增加绝对光标定位序列；直接终端输出策略不变。

## Validation

- backend 测试覆盖 ASCII、ambiguous box drawing、CJK 与 emoji 混排，并断言 tmux 模式每格都有绝对坐标、直接模式保留紧凑输出。
- 滚动条测试覆盖首尾端点、越界 clamp、空格符号与 track/thumb 背景色。
- TUI 全量测试覆盖 Ctrl+F、短帧覆盖、notepad 全屏及既有 Slash/弹窗行为。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
