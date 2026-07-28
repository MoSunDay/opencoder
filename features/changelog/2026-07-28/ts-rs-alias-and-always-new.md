Commit: (working-tree, pre-initial-commit)

# feat(cli): `ts` 新增 `rs` 别名 + 裸 `ts`/`rs` 恒新建 tmux session

## 背景

两点诉求：

1. 期望 `opencode rs --list` / `rs -l` 可用。仓库中并无 `rs` 子命令，唯一带
   `--list`/`-l` 的是 `ts`。`rs` 理解为 `ts` 的短别名最贴合（使 `rs -l`/`rs --list`/
   `rs -r <id>` 全部成立）。
2. 裸 `opencode ts` 不应打开既有 session，而应恒新建一个 tmux 托管 session
   （旧逻辑：恰好 1 个托管 session 时自动重连；>1 时列出并停下）。重连改为
   显式 `ts -r <id>`。

## 变更

### `rs` 别名（Point 1）

- **`crates/cli/src/lib.rs`**：`Ts` variant 增加 `#[command(alias = "rs")]`。于是
  `opencode rs` 解析为 `Ts`，`rs -l`/`rs --list`/`rs -r <id>`/`rs --new` 与 `ts` 同义。
  clap 在 usage 中仍显示规范名 `ts`，帮助文本标注 `rs` 别名。低风险、可一行回退。

### 裸 `ts`/`rs` 恒新建（Point 2）

- **`crates/cli/src/ts/actions.rs::ts_start`**：删除「单托管 session 自动重连」与
  「>1 时列出并停下」两个分支；裸调用恒走 `start_new`。`force_new` 参数保留为
  `_force_new`（向后兼容 `--new`，现为 no-op，已在帮助文本注明）。重连唯一入口为
  `ts -r <id>`（`ts_resume` 不变）。`list_managed`/`attach` 仍分别被 `ts_list`、
  `ts_resume` 使用，无遗留导入。
- **`crates/cli/src/ts/mod.rs`**：模块 doc 头更新为 `ts (alias rs)` + 「恒新建」语义。
- **`crates/cli/src/lib.rs`**：`Ts` doc 与 `--new` doc 注明新语义。

### 文档（最小必要）

- `agents.md`、`agents/cli/index.md`：子命令清单补 `ts 别名 rs`；`Cli` 抽象补
  「裸 ts/rs 恒新建，`-r <id>` 重连」。
- 历史 changelog（`2026-07-21/ts-tmux-session.md` 等）描述当时自动重连行为，为
  时点记录，不改。

## 测试

### 新增（unit / parse）

`crates/cli/tests/cli_parse.rs`：

- `ts_has_rs_alias` — `opencode rs -l` 解析为 `Ts { list: true }`。
- `rs_alias_long_list_flag` — `opencode rs --list` 解析为 `Ts { list: true }`。
- `rs_alias_resume_target` — `opencode rs -r 01HZ` 解析为 `Ts { resume: Some("01HZ") }`。
- `rs_alias_defaults` — `opencode rs` 解析为 `Ts` 默认（list=false / resume=None / new=false）。

### 回归

- `cargo build -p opencoder-cli`、`cargo build --bin opencoder`：clean。
- `cargo test -p opencoder-cli --test cli_parse`：22 passed（4 新增 + 18 既有，含
  原 `ts_subcommand_*` 5 例确认 `ts` 主路径未回归）。
- `cargo clippy -p opencoder-cli --tests`：无 warning。
- 烟雾：`opencoder rs --help`、`opencoder --help`、`opencoder ts --help` 均正确显示别名与新语义。
- `runs_inline` 单测（`ts/mod.rs`）不受影响（其判定不依赖 auto-reattach）。
- e2e（E1–E17）需真实 API key + glm5.2，不在 `cargo test` 范围，未触发；本次为纯
  CLI 解析 + dispatch 语义改动，不触及 LLM/store 路径。

## 风险与回退

- 若 `rs` 别名非用户本意，删除 `#[command(alias = "rs")]` 即回退（Point 1）。
- 若裸 `ts` 应保留自动重连，恢复 `ts_start` 中被删的两个分支即可（Point 2，git 可还原）。
