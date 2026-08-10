Commit: 9f8914a25b734ddab8f13e237362e714524a87dc

# Notepad 搜索结果按工作目录打开

## Context

`rg` 在指定 `current_dir` 中搜索时默认输出相对路径，但 notepad 直接把该路径交给编辑器，导致文件被错误地相对于 opencoder 进程目录读取。搜索可以找到内容，打开结果却显示无法读取。

## Change Summary

- 将 `rg` 命中的相对路径统一解析到 notepad workdir；绝对路径保持不变。
- 路径解析保持为独立纯函数，并增加单元测试和文件搜索集成回归。
- `grep` fallback 的既有路径解析保持不变。

## Validation

- `cargo test -p opencoder-tui --test notepad_search_terminal`。
- `cargo test --workspace`。
- `cargo clippy --workspace --all-targets -- -D warnings`。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
