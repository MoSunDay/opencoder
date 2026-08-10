Commit: d5fb85e

# Notepad vim 撤销/重做（u / Ctrl+R）

## Context

notepad 的 vim 引擎（`crates/tui/src/vim/`）此前没有撤销能力：Normal 模式 `u` 与 `Ctrl+R` 落入 `_ => reset_pending` 空分支，误删文本只能靠 `:q!` 整体丢弃。计划通过共享引擎的挂载点实现 vim 语义的撤销/重做，plan 编辑器顺带获得同样能力。

## Change Summary

- 新增 `crates/tui/src/vim/undo.rs`：`UndoHistory` 双栈（`undo`/`redo` + `insert_session`/`session_recorded` 会话标记）+ 纯函数 `init`/`record_change`/`undo`/`redo`/`after_dispatch`/`maybe_handle_key`，与 `crate::undo`（composer Ctrl+Z/Y）同构、相互独立。
- 插入会话整体算一次撤销（vim 语义）：`i`/`a`/`o`/`c` 进入 Insert 后到 `Esc`/`Ctrl+C` 为止的所有字符、退格、换行折叠为一条历史；`o`/`O`/`c`/`C` 进入会话当键的文本变更并入同一会话。
- `VimState` 增加 `history: UndoHistory` 字段；`VimState::new` 以初始快照为底（`load`/`do_edit`/plan 编辑新建时自动获得全新历史）。
- `vim::handle_vim_key` 包装层：先尝试 `maybe_handle_key`（仅 Normal 且无 `pending_op`/`pending_g` 时拦截 `u`/`Ctrl+R`，消费 count 支持 `2u`/`2Ctrl+R`），否则 dispatch 前后快照对比，`after_dispatch` 记录变更并维护会话边界；`:q!`/`:wq` 等 `VimAction::Exit` 不记录。
- 历史上限 100 条（丢最旧非初始项），新编辑清空 redo；Insert/Command/Search 模式的 `u` 仍正常输入字符（仅 Normal 拦截）。

## Validation

- `cargo test -p opencoder-tui --lib vim`：15 个 `vim::undo` 单测（整会话一次撤销、`o`/`cw` 会话单步、会话内退格、`x` 逐条、`3u`/`2Ctrl+R` 计数、redo 清空、初始态空操作、Insert/Command/Search `u` 输入、pending_op 不拦截、100 条上限）。
- `cargo test -p opencoder-tui --test notepad_edit_flow`：新增 `insert_session_undo_redo_restores_text`、`normal_mode_x_undo_redo` 2 个集成用例，共 7 个全绿。
- `cargo test --workspace` 全量回归通过（2326 passed / 0 failed）；`cargo clippy --workspace --all-targets -- -D warnings` 无告警。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
