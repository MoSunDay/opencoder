Commit: (working-tree, pre-initial-commit)

# feat(tui): 展开态 tool 标题箭头 ▸→▾ 翻转 + 回归测试

## 背景

`ChatView::flatten_with` 渲染 Tool block 时，折叠态与展开态使用相同的前缀箭头
`▸`（U+25B8），用户无法从箭头方向判断工具输出是否展开。展开分支已添加 `[↑]`
折叠提示，但前缀字形未区分。

## 变更

### 行为（1 处，`crates/tui/src/chat.rs`）

`flatten_with` 展开分支（`ChatBlock::Tool { collapsed: false, .. }`）在克隆的
header spans 上将首 span 的前缀由 `▸`（U+25B8）改写为 `▾`（U+25BE）：

- 折叠态：`▸` —— 不变（header 在 `ToolStart` 时以 `▸` 构造，折叠分支不修改）
- 展开态：`▾` —— 新增（仅作用于 `.clone()` 副本，原始 `header` 不受影响）
- 重新折叠：恢复 `▸`（折叠分支直接使用原始 header）

改动隔离于渲染层，不触及 hit-rect / 行数 / 数据形状 / Store / ChatStream。

### 回归测试（1 用例，`crates/tui/src/chat_tests/tool_collapse.rs`）

`expanded_tool_header_prefix_arrow_flips_down`：
- 构造带 output 的 Tool block
- 折叠态断言首 span 以 `▸`（U+25B8）开头
- `toggle_tool_at` 展开后断言以 `▾`（U+25BE）开头
- 再次折叠断言恢复 `▸`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 展开态箭头 ▾ / 折叠态箭头 ▸ / 重折叠恢复 ▸ | `expanded_tool_header_prefix_arrow_flips_down` | `crates/tui/src/chat_tests/tool_collapse.rs` |

### 全量回归

| 检查 | 结果 |
|------|------|
| `cargo test -p opencoder-tui` | PASS — **567 lib + 21 集成 passed; 0 failed; 0 ignored** |
| `cargo clippy -p opencoder-tui --all-targets -- -D warnings` | PASS — 零警告 |
| `cargo build -p opencoder-tui` | PASS — Finished |
| 防修绿扫描 | PASS — 无 `#[ignore]`、无删测试、无弱断言、无调试输出 |

> 验证方式：基于 HEAD（`f3c4aa7`）独立 worktree，仅应用本变更的 2 个源文件，
> 实跑 `cargo test --workspace` 全绿（0 failed）。

## Impact Surface

- 展开的 tool block 标题箭头从 ▸ 变为 ▾，符合「向下展开」的视觉直觉。
- 不影响：折叠态外观、drain 语义 / Store / ChatStream / runner / web / cli。

## 行数

- `crates/tui/src/chat.rs`：778 行（< 800 迭代中上限）
- `crates/tui/src/chat_tests/tool_collapse.rs`：459 行（< 800）
