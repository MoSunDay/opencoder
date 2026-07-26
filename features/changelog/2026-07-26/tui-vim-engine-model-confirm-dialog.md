Commit: (working-tree, pre-initial-commit)

# feat(tui): plan 编辑器内嵌完整 vim 引擎 + /model 保存为默认时弹出居中确认框

## 背景

### `/model`「保存为默认?」确认无可见反馈
`/model` 选定模型后询问「保存为默认?」时，状态机（`model_menu/list.rs`）已持有
`confirm_save_default` 状态，但 view 层从未为其渲染任何 UI——用户看不到提示，只能凭
记忆按 `y/n`。本次在 `model_menu/view.rs` 新增 `render_save_default_confirm()`，
以 62×5 居中黄边弹出框渲染模型名 + `[y] global  [n]/Enter session-only  Esc cancel`，
覆盖在菜单列表之上。状态机逻辑不变。

### plan 编辑器的 vim 只是一段桩代码
此前 `plan_edit.rs` 仅实现极简的「插入/退格/换行」线性输入，号称 vim 却无 Normal 模式、
无 motions / operators / counts / search。本次以全新 `crates/tui/src/vim/` 子模块替换该桩，
实现一套覆盖 Normal / Insert / Command-line / Search 四模式的编辑引擎，并由
`plan_edit.rs` 退化为薄适配器（348→207 行）保留 `PlanEdit`/`PlanEditAction`/
`handle_plan_edit_key` 契约不变。

### app.rs 超过 800 行上限
`app.rs` 已达 826 行（含历史遗留的 `run_app` 巨函数），越过仓库「迭代中文件 ≤ 800」规则。
本次将渲染逻辑抽出为独立 `frame.rs`、把 `flash_visible` 迁至其自然归属（与
`MODE_FLASH_TICKS`/`flash_status_text` 同处），并将 `resume_hint`/`startup_endpoint`/
初始 `ChatView` 构建下沉至 `app_helpers.rs`，使 `app.rs` 降至 791 行。

## vim 引擎能力（plan 模式 + idle + 空输入时 Shift+I 进入）

- **四模式**：Normal / Insert / Command-line / Search。
- **Motions**：`h j k l`、`w b e`、`0 ^ $`、`G gg`（带 count）。
- **Operators**：`d c y`（charwise / linewise，支持 count 与 motion 组合，如 `d3w`、`2dd`）。
- **计数**：任意前缀 count（`5j`、`3x`、`2dd`）。
- **搜索**：`/` 向下、`?` 向上、`n` / `N` 重复 / 反向。
- **行操作**：`x X`、`D C`、`o O`、`J`、`p P`（linewise/charwise 粘贴）、`r`。
- **命令行**：`:q! :q`（丢弃，恢复原文，`is_modified()`→false）、`:w`（标记已存）、
  `:wq :x`（保存并退出，保留文本）。
- **退出语义**：`:q!`/`:q`/Ctrl+C = 丢弃（恢复原始文本）；`:wq`/`:x`/Enter = 保存退出。
- 复用 `composer.rs` 的纯函数（`insert_char`/`backspace`/`move_cursor_vertical`/
  `insert_newline`/`char_width` 等），`prompt_w` 全程硬编码为 2，不重复实现 wrapping/cursor 数学。
- **明确不含**：具名寄存器、宏（`.`/`q`）、visual-block、`:%s` 替换。

## 模块结构

| 文件 | 行数 | 职责 |
|------|------|------|
| `vim/mod.rs` | 394 | 顶层 `VimState`、`VimMode`、入口 `apply_key` |
| `vim/state.rs` | 197 | 不可变快照、cursor 边界、寄存器 |
| `vim/motion.rs` | 311 | 所有 motion 计算（含 word-motion、行边界） |
| `vim/insert.rs` | 194 | Insert 模式：插入/退格/换行/方向键 |
| `vim/normal.rs` | 197 | Normal 模式按键分派 |
| `vim/command.rs` | 190 | Command-line 模式解析（`:wq :q!` 等） |
| `vim/search.rs` | 198 | `/ ? n N` 搜索与高亮跳转 |
| `vim/ops.rs` | 346 | operators（d/c/y）与 yank/delete 寄存器 |
| `vim/actions.rs` | 344 | 行操作（`x X o O J p P r D C gg G`） |

> 全部 ≤ 400 行（新文件限制）。

## app.rs 瘦身 / frame 抽出

- 新增 `crates/tui/src/frame.rs`（111 行）：从 `app_loop.rs` 抽出 `render_frame` 及
  flash 辅助（`flash_status_text`/`copy_status_text`/`MODE_FLASH_TICKS`），并接收迁入的
  `flash_visible`（原定义于 `app_loop.rs`、经 `app.rs` 中转 re-export）。
- `app.rs`：删除误置的 `MODE_FLASH_TICKS` 残留文档注释、删除 `flash_visible` re-export、
  将 `resume_hint`/`startup_endpoint` 下沉 `app_helpers.rs`、把 `run_app` 内 17 行
  「resumed session → 重建 ChatView」内联块抽成 `app_helpers::initial_chat_view`。
  `app.rs` 826→791；`app_loop.rs` 811→705；`app_helpers.rs` 682→721。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **/model 确认框可见** | `save_default_confirm_renders_visible_dialog` | `model_menu/tests/provider_tests.rs` |
| vim Normal: hjkl + count | `actions_tests::hjkl_and_count` | `vim/actions.rs` |
| vim motions: `0 ^ $ gg G` | `actions_tests::zero_caret_dollar` / `gg_and_big_g` | `vim/actions.rs` |
| vim word motions | `actions_tests::word_motions` | `vim/actions.rs` |
| vim operators (d/c/y + count) | `actions_tests::operator_pending_set` / `big_d_and_big_c` | `vim/actions.rs` |
| vim `D C o O` | `actions_tests::o_and_big_o_open_lines` | `vim/actions.rs` |
| vim `x X p P`（行/字符级粘贴） | `actions_tests::x_and_big_x` / `linewise_paste_restores_line` | `vim/actions.rs` |
| vim 搜索 `/ ? n N` | `actions_tests::search_repeat_n_big_n` | `vim/actions.rs` |
| vim Insert: 插入/退格/Ctrl+C 丢弃 | `insert::tests::inserts_chars_and_advances_cursor` / `ctrl_c_discards_and_exits` | `vim/insert.rs` |
| vim `:wq :x` 保存退出 | `command::tests::wq_and_x_save_and_exit_keeping_text` | `vim/command.rs` |
| vim `:q! :q` 丢弃退出 | `command::tests::q_discards_and_exits` | `vim/command.rs` |
| plan_edit 适配器：enter 保存 | `plan_edit::tests::enter_saves_and_exits` | `plan_edit.rs` |
| plan_edit：`:q!` 丢弃恢复原文 | `plan_edit::tests::q_bang_discards_and_exits_unmodified` | `plan_edit.rs` |
| plan_edit：`:wq` 保存 | `plan_edit::tests::wq_saves_and_exits` | `plan_edit.rs` |
| plan_edit：dd 删行 / 搜索导航 | `plan_edit::tests::dd_deletes_current_line` / `search_navigates_cursor` | `plan_edit.rs` |
| flash 可见性窗口（迁入 frame 后） | `flash_visible_within_window` / `flash_visible_expired` / `flash_visible_handles_wraparound` | `app_tests.rs` |

> vim 子模块 94 项单测；model_menu 52 项；plan_edit 14 项。

- 全量回归：`cargo test --workspace` → 当次实跑确认全绿：**1175 passed, 0 failed, 0 ignored**。
- clippy：`cargo clippy --workspace --all-targets` → 零 warning（修复 `frame.rs` 两处 `bool::then` 闭包→`then_some`、`motion.rs` `filter().next()`→`find()`）。
- 构建告警：`cargo build --workspace --tests` → 零 warning / 零 error。
- 行数（新文件 ≤ 400，迭代中文件 ≤ 800）：`vim/*.rs` 全部 ≤ 394；`plan_edit.rs` 222；
  `view.rs` 491；`frame.rs` 111；`app.rs` 791；`app_loop.rs` 705；`app_helpers.rs` 721。
