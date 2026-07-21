# opencode ts：把 TUI 跑进 tmux，SSH 断开后任务继续存活

## 背景

默认 `opencode tui` 直接在前台终端跑。SSH 一断（网络抖动、合上笔记本、重拨），前台进程收到 SIGHUP 退出，正在进行的对话与 subagent 任务随之丢失。本次新增 `opencode ts` 子命令，提供「断线存活」形态：TUI 放进 tmux 会话，SSH 断开后 tmux 会话 detach 但继续运行，重连后一条命令回到原界面。

tmux 逻辑**仅在该子命令路径上触发**；`tui` / `run` / `server` / `client` 等其它命令完全不受影响。

## 设计要点

- **命名约定**：每个受管 tmux 会话名为 `opencode-<ulid>`，其中 `<ulid>` 同时是一个真实的 opencode 会话 id（启动前先在 store 里 seed 一条空 meta）。tmux 名与会话 store 共享同一个稳定 id，`ts -l` 能展示 `/task` 风格信息，`ts -r <id>` 无歧义解析。
- **安全**：所有 tmux 参数通过 `Command::arg(...)` 逐个传递；tmux 用 `execvp` 执行 pane 命令（不走 shell），会话名无法注入 shell 元字符。
- **路径解析**：tmux 子进程里跑的 `opencoder` 用 `std::env::current_exe()` 解析成绝对路径，避免 tmux server 的 PATH 与登录 shell 不一致。
- **`ts -l` 列布局**（每行 `* tmux-name | tmux-id | started | workdir | task`）：
  - `workdir` 列取自 tmux 的 `#{pane_current_path}`（会话当前 pane 的绝对路径），经 `abbreviate_path` 把 `$HOME` 缩成 `~` 以收敛列宽。该列反映 tmux 实际工作目录，与会话是否在当前 store 的 workdir 内互补。
  - `task` 列取 store 里该会话最新的 `/task` 预览（无预览则 title），经 `task_head(raw, 10)` 截取**前 10 个字符**——按 Unicode `char` 计数、先 `trim()` 再取前 N、不加省略号，使「10 字符」计数精确，便于一行并排展示多个会话的当前任务。

## 变更

### 新增 `Command::Ts` 变体（`crates/cli/src/lib.rs`）

`Ts { list: bool, resume: Option<String>, new: bool }`：

- `ts`（无 flag）：若当前恰有一个受管会话则自动 reattach（最常见的「重连」场景）；否则新建。`--new` 强制新建。
- `ts -l`：列出所有 `opencode-*` 受管 tmux 会话，按上面的列布局输出（tmux 名 / `$index` / 相对时间 / attached 标记 / workdir / task 前 10 字符），并用当前 workdir 的 store 补 task 预览；不在当前 workdir 的会话 task 列显示 `(store not in this workdir)`。
- `ts -r <id>`：reattach。`<id>` 接受三种形态：`opencode-<id>` 全名、裸 opencode ulid（自动加前缀）、tmux `$<index>`。已在 tmux 内时 `switch-client`，否则 `attach-session`。

### 新增 `crates/cli/src/ts/` 模块目录（纯函数式，模块级自由函数，无 class）

原 `ts.rs`（508 行，超 400 行单文件上限）按职责拆为子模块，每个文件 ≤400 行：

- `ts/mod.rs`：入口 `ts_dispatch`（list / resume / start 三分支）+ 纯函数 `runs_inline`（「已在 tmux 内且既非 `-l` 也非 `-r` → 内联跑 TUI、绝不嵌套 tmux」的判定，便于单测）+ `inside_tmux`/`tmux_available` 再导出。
- `ts/env.rs`：`tmux_available` / `which_tmux` / `inside_tmux`。
- `ts/naming.rs`：`TMUX_PREFIX` / `session_name` / `id_from_name` / `fresh_id` / `resolve_target`。
- `ts/tmux.rs`：tmux 进程管道 `tmux_bin` / `tmux_inherit` / `session_exists` / `attach` + `ManagedSession` 数据模型 / `parse_list_line` / `list_managed`。
- `ts/actions.rs`：`ts_start` / `start_new` / `ts_list` / `ts_resume` / `ensure_session` / `open_store_for` / `current_workdir`。
- `ts/display.rs`：纯展示辅助 `now_secs` / `format_ts` / `task_head` / `abbreviate_path`。

### dispatch（`src/main.rs`）

- `is_tui` 纳入 `Command::Ts`，日志走文件（避免 attach 后污染 TUI）。
- 新增 `Command::Ts` 分支：内联判定收敛为纯函数 `opencoder_cli::ts::runs_inline(*list, resume.is_some(), inside_tmux())`——已在 tmux 内且无 flag 则**内联跑 TUI**（绝不嵌套），否则交给 `ts_dispatch`。

### 解析与分发覆盖（`crates/cli/tests/cli_parse.rs`）

新增 `Command::Ts` 解析测试：`ts -l`→`Ts{list:true}`、`ts -r <id>`→`Ts{resume:Some(..)}`、`ts --new`→`Ts{new:true}`、`ts`（无 flag）→全默认。分发分支的纯判定另由 `runs_inline` 单测覆盖。

## 已知限制（v1）

- 一个 tmux 会话内可通过 `/task` 切换多个 opencode 会话；tmux 名只记录「启动会话」，会随 `/task` 切换过期。`ts -l` 显示的是 launch session；精确的「当前会话」需 TUI 上报状态，超出本次范围。
- `ts -l` 只能用**当前 workdir** 的 store 富化 task 列；跨 workdir 的受管会话显示 `(store not in this workdir)`。`workdir` 列本身来自 tmux 的 `#{pane_current_path}`，不受此限制。
- 没装 tmux 时 `opencode ts` 直接报错（语义清晰）；需普通 TUI 用 `opencode tui`。

## 验证

- `cargo build -p opencoder-cli`（编译干净）。
- `cargo test -p opencoder-cli`：lib 单测 27 passed（含 ts 模块 8 个纯函数单测：naming 3 / tmux 1 / display 3 / mod `runs_inline` 1），`tests/cli_parse.rs` 15 passed（含 4 个 `ts_subcommand_*` 解析测试），`tests/fork_session.rs` 3 passed。
- `cargo clippy -p opencoder-cli --all-targets -- -D warnings`（ts 子模块零告警）。
- 手动（真实 tmux 3.2a）：`ts -l`（无 server → 空列表）；造一个 `opencode-<fake>` detached 会话后 `ts -l` 正确列出并富化 workdir/task 列；`ts -r <bare ulid>` 正确解析并尝试 attach（无 TTY 时 attach 失败属预期）。
- 全量 `--workspace` 级 test/clippy 当前被**范围外**的并发 subagent 重构 WIP（`crates/session/src/resume.rs`、`crates/tui/src/chat.rs` 等）阻断，非本特性代码；待其落定后在 live repo 重跑 `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` 取干净证据。
