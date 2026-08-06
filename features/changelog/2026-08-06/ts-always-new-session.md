Commit: (working-tree, pre-initial-commit)

# `opencode ts` 始终创建新会话

## 背景
裸 `opencode ts`（别名 `rs`）此前会**隐式重用**最近活跃的 tmux 会话——无论其工作目录是否匹配。在仓库 B 中运行 `ts` 可能附加到仓库 A 中创建的会话，表现为「启动失败」。修复后，每次裸 `ts` 都获得一个全新会话；唯一保留的附加路径是显式 `ts -r <id>`（或 `--session <id>` 且该 tmux 实例已存在）。`ts -l`（Store 优先）已统一列出当前仓库的全部会话，不受影响。

## 变更

### cli: 移除裸 `ts` 的隐式重用分支
- **`crates/cli/src/ts/actions.rs`**（`ts_start`）：删除 `else if let Some(live) = list_managed()?.into_iter().next()` 重用分支及 `force_new` 参数；裸 `ts` 现在总是 `ensure_session` + `spawn_session`。仅当 `--session <id>` 指定的 tmux 会话已存在时才附加。
- 新增纯函数 `explicit_attach_target(session_arg, exists)` 锁定该契约：无 `--session` 恒返回 `None`；`--session`+已存在才返回 `Some(name)`。

### cli: 移除 `--new` 标志（行为已成默认）
- **`crates/cli/src/lib.rs`**（`Command::Ts`）：删除 `new` 字段；更新帮助文档为「A bare `ts`/`rs` **always creates a fresh session**」。
- **`crates/cli/src/ts/mod.rs`**（`ts_dispatch`）：签名移除 `force_new`。
- **`src/main.rs`**：`Command::Ts` 解构与两处 `ts_dispatch` 调用同步移除 `new`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 裸 `ts` 不附加（无 session） | explicit_attach_target_bare_ts_returns_none | crates/cli/src/ts/actions.rs |
| `--session`+已存在才附加 | explicit_attach_target_session_exists_attaches | crates/cli/src/ts/actions.rs |
| `--session`+不存在不附加 | explicit_attach_target_session_not_live_returns_none | crates/cli/src/ts/actions.rs |
| `ts --new` 被拒绝（UnknownArgument） | ts_subcommand_rejects_new_flag | crates/cli/tests/cli_parse.rs |
