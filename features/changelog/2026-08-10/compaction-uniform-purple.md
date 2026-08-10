Commit: 760bc53497421504292163401fa7433cf3fc9be4

# Compaction 标签与正文统一紫色

## Context

Compaction 标签是加粗紫色，展开正文沿用默认文字色；在 16 色终端中 BOLD 还可能把标签提升为亮紫色，形成两种不同颜色。

## Change Summary

- TUI 标签保持首字母大写的 `Compaction`。
- 标签和展开正文统一使用 `theme::local_color()`。
- 标签移除 BOLD，保证与正文严格同色。

## Impact Surface

只改变 TUI Compaction 折叠块的视觉样式；headless CLI、事件、配置和持久化格式不变。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
