Commit: 6f7aea4c9c0879fbb497d778955a1f1098b29600

# Notepad vim 撤销/重做（u / Ctrl+R）

## Context

notepad 的 vim 引擎（`crates/tui/src/vim/`）此前没有撤销能力：Normal 模式 `u` 与 `Ctrl+R` 落入 `_ => reset_pending` 空分支，误删文本只能靠 `:q!` 整体丢弃。计划通过共享引擎的挂载点实现 vim 语义的撤销/重做，plan 编辑器顺带获得同样能力。

## Change Summary

- 新增 `crates/tui/src/vim/undo.rs`：`UndoHistory` 双栈（`undo`/`redo` + `insert_session`/`session_recorded` 会话标记）+ 纯函数 `init`/`record_change`/`undo`/`redo`/`after_dispatch`/`maybe_handle_key`，与 `crate::undo`（composer Ctrl+Z/Y）同构、相互独立。
- 插入会话整体算一次撤销（vim 语义）：`i`/`a`/`o`/`c` 进入 Insert 后到 `Esc`/`Ctrl+C` 为止的所有字符、退格、换行折叠为一条历史；`o`/`O`/`c`/`C` 进入会话当键的文本变更并入同一会话。
- `VimState` 增加 `history: UndoHistory` 字段；`VimState::new` 以初始快照为底（`load`/`do_edit`/plan 编辑新建时自动获得全新历史）。
- `vim::handle_vim_key` 包装层：先尝试 `maybe_handle_key`（仅 Normal 且无 `pending_op`/`pending_g` 时拦截 `u`/`Ctrl+R`，消费 count 支持 `2u`/`2Ctrl+R`），否则 dispatch 前后快照对比，`after_dispatch` 记录变更并维护会话边界；`:q!`/`:wq` 等 `VimAction::Exit` 不记录。
- 历史上限 100 条（丢最旧非初始项），新编辑清空 redo；Insert/Command/Search 模式的 `u` 仍正常输入字符（仅 Normal 拦截）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 插入会话整体一次撤销 | `insert_session_undoes_whole_session` | `crates/tui/src/vim/undo.rs` |
| `o` 会话单步 | `o_session_is_one_step` | `crates/tui/src/vim/undo.rs` |
| `cw` 会话单步 | `cw_session_is_one_step` | `crates/tui/src/vim/undo.rs` |
| 会话内退格折叠 | `backspace_inside_session_collapses` | `crates/tui/src/vim/undo.rs` |
| `x` 每 `u` 撤一步 | `x_undoes_one_edit_per_u` | `crates/tui/src/vim/undo.rs` |
| 计数撤销（`3u`） | `count_undo_undoes_multiple_steps` | `crates/tui/src/vim/undo.rs` |
| `Ctrl+R` 重做 | `ctrl_r_redoes_undone_edit` | `crates/tui/src/vim/undo.rs` |
| 计数重做（`2Ctrl+R`） | `count_redo_redoes_multiple_steps` | `crates/tui/src/vim/undo.rs` |
| 新编辑清空 redo | `new_edit_clears_redo` | `crates/tui/src/vim/undo.rs` |
| 初始态撤销空操作 | `undo_at_initial_is_noop` | `crates/tui/src/vim/undo.rs` |
| Insert 模式 `u` 正常输入 | `u_types_normally_in_insert` | `crates/tui/src/vim/undo.rs` |
| Command 模式 `u` 输入 | `u_types_into_cmdline_in_command_mode` | `crates/tui/src/vim/undo.rs` |
| Search 模式 `u` 输入 | `u_types_into_search_input` | `crates/tui/src/vim/undo.rs` |
| pending_op 不拦截 `u` | `pending_op_defers_u` | `crates/tui/src/vim/undo.rs` |
| 历史上限 100 条 | `history_caps_at_100_steps` | `crates/tui/src/vim/undo.rs` |
| 整会话撤销/重做恢复文本（集成） | `insert_session_undo_redo_restores_text` | `crates/tui/tests/notepad_edit_flow.rs` |
| Normal `x` 撤销/重做（集成） | `normal_mode_x_undo_redo` | `crates/tui/tests/notepad_edit_flow.rs` |

- 定向单测：`cargo test -p opencoder-tui --lib vim` → 15 个 `vim::undo` 单测全绿；`cargo test -p opencoder-tui --test notepad_edit_flow` → 7 个全绿（新增 2）。
- 全量回归：`cargo test --workspace` → **2326 passed / 0 failed**。数字取证：隔离 worktree（6f7aea4 当次）复跑 `cargo test --workspace` 实测 2326 passed / 0 failed（145 个测试二进制），与算术闭环一致（2309 隔离基线 + 本批次 17 项新测试 = 2326）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 无告警。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
