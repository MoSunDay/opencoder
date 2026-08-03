# fix(tui): 恢复折叠块 header 的 `(N lines)` 行数摘要（Thinking + Compaction）

## 背景

2026-08-03 的 `compaction-click-expand` 变更在为 Compaction 块接入 click-to-expand
管线时，顺手「清理」了共享函数 `render_collapsible` 的折叠 header：移除了
`(N lines)` 行数摘要（同时移除了 `[↓ expand]` / `[↑ collapse]` 提示词）。该改动
通过共享函数同时影响了 **Thinking 块**——其折叠态的行数摘要被静默删除，而当时
**没有任何测试覆盖 Thinking header 的 `(N lines)` 文本**，这是删除未被发现的根因。

原始格式（2026-07-05 文档）只记录 `(N lines)` 行数，**无** 提示词。本次仅恢复行数
摘要，不恢复提示词——两者除图标外完全一致（本就共用同一函数）。

## 变更

### `crates/tui/src/compaction_block.rs` — 补回行数（核心）

`render_collapsible`（Thinking 与 Compaction 共用）：

- **折叠分支**：计算 `let n = text.lines().count();`，header 文字由
  `format!("{icon} {label}")` → `format!("{icon} {label} ({n} lines)")`。
  效果：折叠态显示 `💭 Thinking (3 lines)` / `🗜 Compaction (2 lines)`。
- **展开分支不动**：header 仍为 `{icon} {label}` italic-bold，不带行数。
- 同步更新文档注释：「折叠态显示 `(N lines)` 行数摘要」。

折叠态渲染行数仍为 **1 行**（header 文字变长但不增行），故 `collect_headers` /
`header_line_idx` / full-width hit-rect 布局全部不变——头部命中管线无副作用。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Compaction 折叠 header 显示行数 | `collapsed_header_shows_line_count` | `render_tests/compaction.rs` |
| Compaction 折叠显示行数、展开不带行数 | `header_text_shows_line_count` | `chat_tests/compaction_state.rs` |
| **Thinking 折叠 header 显示行数、展开不带行数（新增）** | `thinking_header_shows_line_count_when_collapsed` | `chat_tests/thinking_state.rs` |

- `collapsed_header_has_no_extra_text` → 翻转为 `collapsed_header_shows_line_count`
  （断言 `contains("2 lines)`）。
- `header_text_is_clean` → 翻转为 `header_text_shows_line_count`（断言
  `contains("3 lines")`，并新增展开态 `!contains("lines")` 断言）。
- 新增 Thinking header 行数测试（此前无任何覆盖）。
- 命中管线相关测试（hit-rect / header_line_idx / 展开行不变）全绿，确认布局无回归。

## 验证

- `cargo test -p opencoder-tui` -> TUI 域 **863 passed / 0 failed**（含改动 2 + 新增 1）。
- `cargo test --workspace` -> 本次变更的 TUI 域全绿；workspace 偶发 **与本变更无关的
  session 失败**（flaky/drift：哪条用例失败随运行而变，时为 0 失败、时为 1-2 条，
  如 `resume_and_replay_*` 或 `timeout_marks_subagent_cancelled`），均位于
  `opencoder-session` crate。该 crate **不依赖** `opencoder-tui`（crate DAG 单向
  tui->session），故 TUI 渲染变更不可能影响这些用例；失败源自工作树并发的
  session/store/web/cli 脏改动（见下方"范围外改动"），非本次变更引入。
- `cargo clippy --workspace --all-targets -D warnings` -> 零警告。
- `cargo build --workspace` -> 编译干净。

## 范围外改动（不属本次提交）

工作树存在并发的 session/store/web/cli 修改，已识别并排除出本次提交范围
（`crates/{cli,session,store,web}/**`）。它们偶发引入 session 失败测试（flaky，哪条
失败随运行而变），与本次 TUI 渲染微调无关。提交时仅 stage 本任务的 4 个 TUI 文件 + 本 changelog。

## Impact Surface

- **行为变更**：Thinking 与 Compaction 折叠态 header 重新显示 `(N lines)`；展开态不变。
- **无布局风险**：行数仅让 header 文本变长，折叠态渲染行数仍为 1，`header_line_idx`
  与 full-width hit-rect 计算完全不受影响。
- **接缝不变**：仅触 TUI 渲染 + 测试，不碰 `Store` / `ChatStream` / session / core /
  llm / store 任何 API 表面。
- **空文本边界**：`"".lines().count()` = 0 → 显示 `(0 lines)`，与原始行为一致。

## Related Docs

- [agents/tui](../../agents/tui/index.md) — render_collapsible、命中管线
- 回滚前序变更：[2026-08-03/compaction-click-expand](compaction-click-expand.md)
