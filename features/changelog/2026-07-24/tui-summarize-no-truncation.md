Commit: (working-tree, pre-initial-commit)

# fix(tui): summarize() no longer truncates bash commands to 80 columns

## 背景
TUI transcript 中工具调用（如 bash）的 header 行由 `summarize()` 渲染。
旧实现用 `short(s, 80)` 把命令截断到 80 显示列并加省略号 `…`，导致真实命令
被隐藏在省略号之后——用户无法看到实际执行了什么。transcript body 层本就用
`Paragraph::wrap(Wrap { trim: false })` 按终端实际宽度自动换行，截断是多余的。

## 变更
### summarize 返回完整文本
- **`crates/tui/src/chat.rs`** `summarize()`：
  - 对 `command` / `path` / `description` / `pattern` / `prompt` 字段，
    返回 `s.trim().to_string()`（完整文本）而非 `short(s, 80)`
  - fallback（序列化 JSON / 非对象值）同样改为返回完整 trimmed 文本

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 长 bash 命令不被截断 | `summarize_keeps_full_bash_command_no_truncation` | chat_tests.rs |

- 全量回归：`cargo test --workspace` → 872 passed; 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净编译
- 行数：chat.rs (本文件远 < 800)、chat_tests.rs (本文件远 < 800)

## Impact Surface
- TUI transcript 工具调用 header 不再截断，长命令完整可见并由 body 层换行。
- 不影响：CLI / Web / session / store / LLM 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
