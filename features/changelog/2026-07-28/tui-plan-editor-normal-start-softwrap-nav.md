Commit: (working-tree, pre-initial-commit)

# fix(tui): plan 编辑器 Normal 起始 + 软换行方向键导航 + 边框/提示修正

## 背景

TUI plan-text 编辑器（Shift+I 进入）存在四个体验问题：

1. **起始模式不正确**：`PlanEdit::new` 未强制 Normal 模式，编辑器可能以 Insert
   模式打开，用户的首个按键被当作文本插入而非 vim 命令，与「先定位再编辑」的
   vim 心智模型冲突。
2. **边框无模式标识**：plan 编辑器边框缺少当前 vim 模式（NORMAL/INSERT）提示，
   用户无法判断当前所处的模式。
3. **模式切换提示过长**：进入 plan 模式时的闪现文本冗长，占据过多注意力。
4. **方向键与软换行冲突**：Up/Down 在输入超过一行（软换行）时仍走「历史记录翻页」
   分支，导致光标无法在可视多行间移动，反而把当前输入替换成历史命令。

## 变更

### 行为修正（4 处）

- **`crates/tui/src/plan_edit.rs`** — `PlanEdit::new` 显式 `vim.mode = VimMode::Normal`
  并将光标重置到 0、`modified = false`，保证编辑器始终以 Normal 模式、光标在行首、
  未修改状态打开。
- **`crates/tui/src/render.rs`** — plan 编辑器边框增加 `.title(" edit plan ")` 顶部
  标题与 `.title_bottom(Line::from(format!(" {label} ")).alignment(Left))` 底部模式
  标签，实时反映 NORMAL/INSERT 模式。
- **`crates/tui/src/app_loop.rs`** — 进入 plan 模式的闪现文本精简为 `"→ plan mode"`。
- **`crates/tui/src/key_handler.rs`** — Up/Down 的判定条件由「输入非空即翻历史」改为
  `composer::display_rows(input, inner_w, prompt_w) > 1`：仅当输入软换行成多行时，
  Up/Down 才移动光标跨越可视行；否则保持原有历史翻页语义。

### 结构性拆分（满足 800 行文件上限）

`key_handler.rs` 因测试累积达 1030 行（>800）。按职责将内联 `mod tests` 拆出为两个
`#[path]` 子模块，遵循 `app_loop_tests` 既有的拆分约定：

- **`crates/tui/src/key_handler.rs`**：保留 `KeyAction` 枚举 + `handle_key` 调度 +
  `apply_scroll` / `move_hist` 辅助函数（1030 → 385 行）。
- **`crates/tui/src/key_handler_tests.rs`**（新增，315 行）：scroll 分页、disabled-input
  门控、剪贴板（Ctrl+V）、agent 切换 Tab 行为（10 个测试）。
- **`crates/tui/src/key_handler_plan_edit_tests.rs`**（新增，345 行）：历史翻页（`move_hist`）、
  Shift+I plan-edit 进入门控、软换行行导航、Enter → Steer/SubagentSteer 派发（11 个测试）。

> 关联改动：`handle_key` 新增 `subagent_focused` 参数（Enter 在聚焦运行中 subagent 时
> 派发 `SubagentSteer`）。其调用方（`app_loop.rs`、`subagent_input.rs`）已同步更新；
> 该能力属范围外未提交工作，此处仅说明编译期已对齐。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 编辑器以 Normal 模式 / 光标在首 / 未修改 打开 | `new_starts_normal_cursor_at_top_unmodified` | `crates/tui/src/plan_edit.rs` |
| 软换行时 Up/Down 移动光标而非翻历史 | `up_down_navigate_soft_wrapped_rows` | `crates/tui/src/key_handler_plan_edit_tests.rs` |
| 历史翻页保留（回归基线） | `move_hist_up_loads_previous_entry` / `move_hist_down_after_up_restores_blank` | `crates/tui/src/key_handler_plan_edit_tests.rs` |
| Shift+I plan-edit 进入门控（回归基线） | `shift_i_in_plan_mode_idle_enters_plan_edit` 等 4 项 | `crates/tui/src/key_handler_plan_edit_tests.rs` |

- 边框标题（`title`/`title_bottom`）与闪现文本（`"→ plan mode"`）为纯展示层字符串常量，
  未新增 pub fn / 枚举变体 / 端点，由既有渲染路径承载，无独立行为断言需求。

### 全量回归

| 检查 | 结果 |
|------|------|
| `cargo build --workspace` | PASS — Finished |
| `cargo test --workspace` | PASS — **1267 passed; 0 failed; 0 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
| TUI 单测基线 | 566 passed（含 21 项 key_handler 拆分测试，10 + 11） |

防修绿扫描：无新增 `#[ignore]`、无删除测试、无弱断言（`assert!(true)`/`is_ok()`）、
无 `println!`/`dbg!`/`eprintln!`。

## Impact Surface

- plan 编辑器现以 Normal 模式打开，符合 vim 用户预期；边框底部实时显示模式。
- 多行软换行输入可用 Up/Down 在可视行间移动光标；单行时仍为历史翻页（行为不变）。
- 不影响：drain 语义 / Store / ChatStream / runner / web / cli。改动隔离于 TUI 输入与渲染路径。
