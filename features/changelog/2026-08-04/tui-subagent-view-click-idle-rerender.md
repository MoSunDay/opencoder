Commit: (working-tree, pre-initial-commit)

# fix(tui): 鼠标事件后设置 dirty，subagent [view] 点击在空闲态正常进入

## 背景

TUI 事件循环的渲染条件是 `dirty && render_pending`（`crates/tui/src/app.rs`）。
`anim_ticker` 仅在 `running == true` 时设置 `dirty`。

`Event::Mouse` 分支调用 `handle_mouse` 修改了大量状态（`subagent_focus`、
`scroll`、`follow`、`selection`、折叠切换等），但**从未设置 `dirty = true`**。

后果：当 session 空闲（subagent 已完成、无运行中的 turn）时，点击 subagent
header 的 `[→ view]` 虽然正确设置了 `subagent_focus`，但 `dirty` 保持 false，
视图不重绘 —— 用户看到点击毫无反应。subagent 运行中时因 `anim_ticker` 持续
设置 `dirty` 而正常工作，所以此 bug 只在空闲态暴露。

git blame 确认该分支自引入以来一直缺少 `dirty = true`，属长期遗留缺陷。

## 变更

### 鼠标事件后触发重绘 — `crates/tui/src/app.rs`

在 `Event::Mouse(m)` 分支末尾添加 `dirty = true;`，确保每次鼠标交互
（点击、拖拽、滚轮）后立即重绘。与 `Event::Key` / `Event::Paste` 分支
一致——它们在处理完后均设置 `dirty`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| subagent [view] 点击进入（None→Some） | `clicking_subagent_view_enters_subagent` | `crates/tui/src/app_helpers_tests/mouse_tests.rs` |

此前所有 subagent 鼠标测试都以 `subagent_focus` 预置为 `Some` 开头，
不存在从 `None` 点击进入的回归用例；本次补齐。

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`crates/tui/src/app.rs` 800 ≤ 800；`mouse_tests.rs` 624 ≤ 800

## Impact Surface

- **修复**：空闲态下所有鼠标交互的即时反馈——subagent `[→ view]` 进入、
  thinking/tool/compaction 折叠切换、文本选择、滚轮滚动。
- **不影响**：CLI / Web / session / store / llm 边界。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
