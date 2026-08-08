Commit: (working-tree, pre-initial-commit)

# refactor(tui,cli): Thinking/Compaction 块样式化 + CLI 死代码清理

## 背景

1. **TUI**：Thinking 与 Compaction 折叠块此前没有任何逐元素样式——标题行与正文行都
   使用默认文本风格。用户无法在视觉上把 "Thinking" 标题与其正文区分开，标题与正文
   混在同一灰度里，可读性差。
2. **CLI**：`run.rs` 里残留两个 `#[allow(dead_code)]` 标记的死代码桩函数
   （`run_once_inline`、`_duration`），是早期迭代遗留下来的，不再有任何调用点。

## 变更

### `crates/tui/src/compaction_block.rs`（`render_collapsible` 函数）

- 新增两个参数：`header_style: Style` 与 `body_style: Style`，把样式决策上交给调用方。
- 折叠态的标题行（`{icon} {label} ({n} lines)`）改用 `header_style`。
- 展开态的标题行（`{icon} {label}`）改用 `header_style`。
- 展开态的每一行正文（缩进两格）改用 `body_style`。
- 模块注释同步更新为 "callers pick their own palette"。

### `crates/tui/src/chat.rs`（约第 510 行的块渲染分支）

- **Thinking 块**：`header_style = Style::default().fg(theme::accent())
  .add_modifier(Modifier::BOLD)`，`body_style = Style::default().fg(theme::muted())`
  ——标题用 accent + 粗体，正文用 muted，建立视觉层次。
- **Compaction 块**：`header_style` 与 `body_style` 均为 `Style::default()`，保持
  摘要文本为朴素输出（与既有外观一致，不引入额外颜色噪声）。

### `crates/cli/src/run.rs`

- 删除 `run_once_inline` 函数（约第 331–343 行）——只是对 `run_once` 的死代码
  包装桩，无任何调用点。
- 删除 `_duration` 函数——返回 `Duration::from_secs(0)` 的桩，同样无调用点。
- 从 import 中移除 `run_once`（本文件已不再引用它）。
- 一并移除上述两个函数上的 `#[allow(dead_code)]` 标注。

## 测试说明

无需新增测试：样式变更由既有的 TUI 渲染快照/快照类测试覆盖
（`render_collapsible` 的输出形态、Thinking 块与 Compaction 块的渲染路径均已有
既存用例锁定）；死代码删除本身无需测试。

| 关注点 | 覆盖来源 | 层 |
| --- | --- | --- |
| Thinking / Compaction 块渲染形态 | 既有 render/chat 单元测试 | unit(render) |
| `render_collapsible` 折叠/展开分支 | 既有 collapsible 渲染用例 | unit(render) |
| 死代码移除（`run_once_inline` / `_duration`） | 无需测试（纯删除） | — |

## Gate

- 全量回归：`cargo test --workspace` → 全绿（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
  （随删除移除了 `#[allow(dead_code)]` 标注，无新增告警）。
- 构建：`cargo build --workspace` → 编译干净（EXIT=0）。
- 行数：`run.rs` 删除后行数下降；`compaction_block.rs` 仅新增形参，仍远低于 800 行 Gate。

## 影响面

- **用户**：Thinking 块标题以 accent + 粗体显示、正文以 muted 显示，标题与正文的
  视觉层次更清晰；Compaction 块外观不变。
- **不影响**：session / store / web / 任何持久化或协议；无数据库、配置、环境变量
  或公开 schema 变化。
- 纯展示层与本地死代码清理，行为语义零变化。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
- [CLI 模块](../../../agents/cli/index.md)
