Commit: 760bc53497421504292163401fa7433cf3fc9be4

# Slash 复合命令支持空格补全

## Context

命令弹窗会把 `/plan ` 中的空格继续写入过滤条件，导致用户无法自然输入带需求正文的复合命令。

## Change Summary

- 未修饰的 Space 与 Tab 一样补全当前选中的规范命令名，关闭弹窗并由 composer 追加尾随空格。
- 没有匹配命令时 Space 不改变查询，也不关闭弹窗。
- 弹窗帮助明确显示 `Space/Tab=fill`。

## Impact Surface

只影响 slash 命令弹窗的输入补全；Enter 立即执行、Tab 补全和控制命令解析规则保持不变。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
