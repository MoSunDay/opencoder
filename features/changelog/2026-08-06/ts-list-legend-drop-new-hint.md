Commit: (working-tree, pre-initial-commit)

# `ts -l` 列表底部不再提示已移除的 `--new`

## 背景
前序提交（`ts-always-new-session`）移除了 `--new` 标志，使裸 `ts` 恒新建会话。
但 `ts -l` 面板底部的命令提示行仍打印 `new: opencode ts --new`——一个 clap 现已
拒绝（`UnknownArgument`）的死命令。本提交清除此残留，并把提示抽成常量以便回归测试。

## 变更

### cli: `ts_list` 提示行去掉 `--new`
- **`crates/cli/src/ts/actions.rs`**：新增模块级常量 `LIST_LEGEND`（仅含 `resume`/`clean` 两条活命令）；`ts_list` 改为 `println!("{}", LIST_LEGEND)`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 提示行不含已移除标志、保留活命令 | list_legend_has_no_removed_flags | crates/cli/src/ts/actions.rs |
