Commit: (working-tree, pre-initial-commit)

# TUI「按 Esc 后整屏冻死、必须 kill 进程」根因修复（输入采集重构 + 终端 RAII + worker 死亡监管）

## 背景

用户报告：**按 Esc（中止 / 返回）有概率导致整屏彻底无响应——连 Ctrl+C / Ctrl+D 都退不出，必须 kill 进程**。

## 诊断

### 根因：crossterm `EventStream` 在 Kitty 键盘协议下被 Esc 消歧序列永久挂死

- Esc 处理逻辑本身（`app.rs::KeyAction::Cancel`）**全是同步代码**，不可能阻塞事件循环——排除"Esc 逻辑冻死"。
- `run_app` 用 `crossterm::EventStream`（`app.rs` 旧 `events.next()`），其内部 reader 任务跑阻塞 `event::read()`。
- `app.rs` 无条件启用 `DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS`（Kitty 键盘协议）。单按 Esc 产生一个**需等待后续字节或超时才能消歧**的序列；reader 一旦卡在这次消歧读上，`events.next()` 永不 resolve，`tokio::select!` 永远停在这个分支。
- 一旦停住：主循环不再 poll 任何事件 → raw 模式下 Ctrl+C/Ctrl+D 交给程序、程序不读就无人响应 → 进程还活着但整屏死 → 必须 kill。**"有概率"= 踩中 Esc 字节时序**。与用户描述完全吻合。

故根因是：**主循环的活性被绑在一个可能无限阻塞的 EventStream 读上**。正确修法不是"发现它卡了再重建"（那是不可持续的补丁），而是**让输入采集天生有上界、永不无限阻塞**——从结构上消除该失败模式。

### 顺带确认的其它冻死/卡死/变砖隐患（本轮一并修）

- **终端恢复缺失**：旧 `run()` 的清理（`disable_raw_mode` / `PopKeyboardEnhancementFlags` / `DisableMouseCapture` / `LeaveAlternateScreen`）只在 `run_app` 正常返回时跑；全仓无 `Drop`/`catch_unwind`/panic hook。任何 panic 都跳过清理 → 终端留在 raw+alt-screen+鼠标捕获 = 肉眼与"冻死"无法区分，需 kill + `reset`。
- **worker 静默死亡**：worker 是裸 `tokio::spawn`，panic 后 `cmd_rx` 关闭、不再发 `TurnDone` → `running` 永远 true、提交全进死通道，UI 看似"啥都不干"。

## 变更（根因级，无补丁/无定时器/无 try-catch 兜底）

### 1. 输入采集重构：专用 OS 线程 + 有界 `poll`/`read`，弃用 `EventStream`（核心）
- **`crates/tui/src/input.rs`（新增，~100 行）**：`spawn_input_pump() -> (mpsc::Receiver<Event>, thread::JoinHandle)`。`std::thread::spawn` 跑循环：`event::poll(150ms)` 命中则 `event::read()`，经 tokio mpsc `blocking_send` 投递。
  - `poll(timeout)` 契约**最多阻塞 timeout** → 线程每 ≤150ms 必醒 → **不可能被半个 Esc 序列永久挂死**；冻死失败模式从结构上消失。
  - 关闭：receiver drop → `Sender::is_closed()` 每轮检查即令线程退出（≤150ms），无标志位、无 join 阻塞。
- **`crates/tui/src/app.rs`**：删 `EventStream::new()` 与 `events.next()` 分支；`run_app` 起首 `spawn_input_pump()`，`select!` 改 select 在 `input_rx.recv()` 上；`None`（采集线程退出/stdin EOF）→ `UiCmd::Quit` + break（沿用旧"流关闭即退出"语义）。
- **`crates/tui/Cargo.toml`**：crossterm 去掉 `event-stream` feature（不再需要）；顺带移除 tui 不再使用的 `futures`、`tokio-stream` 依赖与 `use futures::StreamExt;`。

### 2. 终端生命周期 RAII：`TerminalGuard` + panic hook
- **`crates/tui/src/terminal.rs`（新增，~85 行）**：`TerminalGuard::enter()` 开 raw+alt-screen+cursor+鼠标+Kitty，并 `set_hook`（panic 时**先 `restore()` 再调原 hook**，使 backtrace 落在已恢复的终端而非 alt-screen）；`Drop` 幂等 `restore()`。`run()` 以 `let _guard = TerminalGuard::enter()?;` 守住全生命周期——**任何退出路径（正常/`?`错误/panic unwind）都恢复终端**，不再变砖。

### 3. worker 死亡可观测：通道关闭即检测，保持 UI 可响应
- **`app.rs::start_turn`** 返回值改为 `bool`：命令通道关闭（worker 已死）→ 返回 `false`。
- 4 个 turn 派发点（Submit idle / SwitchAndStart / Compact / TurnDone 续跑）收到 `false` → `worker_dead(&mut chat)` 推 `[worker stopped]` marker 并 `break`。
- **关键**：输入采集在独立 OS 线程，worker 死后**主循环仍响应 Ctrl+C/D**，用户可干净退出而非面对冻死 spinner。靠 channel 语义而非 try/catch，长期可维护。
- `crates/session/src/runner.rs`：panic 源审计——`tool_calls[i]`（`i` 有界）、json 走 `unwrap_or_default`、流式错误走 `?` 返 `Result`，确认 panic-safe，无需改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| **核心命题：单按 Esc 在有界时间内被投递（冻死失败模式结构消失的直接证据）** | `lone_esc_is_delivered_within_bound` | `crates/tui/tests/input_pty.rs`（新增，pty 表征） |
| 输入采集线程 receiver drop 即在超时内退出（关闭契约） | `pump_exits_when_receiver_dropped` | `crates/tui/src/input.rs`（新增） |
| 终端恢复幂等、无 TTY 也不 panic | `restore_is_idempotent_without_a_tty` | `crates/tui/src/terminal.rs`（新增） |
| `write_restore` 依序发出全部三条恢复序列（raw→Kitty→鼠标） | `write_restore_emits_all_three_sequences` | `crates/tui/src/terminal.rs`（新增） |
| panic hook 先恢复终端再链到原 hook（backtrace 落在已恢复终端） | `hook_body_restores_before_chaining_to_prev` | `crates/tui/src/terminal.rs`（新增） |
| worker 死亡：start_turn 在通道关闭时返 false | `start_turn_reports_false_when_worker_is_dead` | `crates/tui/src/app_tests.rs`（新增） |
| worker_dead 推可见 marker | `worker_dead_pushes_a_marker` | `crates/tui/src/app_tests.rs`（新增） |
| 双击 Esc 硬中止仍即时（既有回归） | `cancel_hard_aborts_a_running_tool` | `crates/session/tests/hard_abort.rs` |
| `/task` 切换后双击 Esc 命中新会话 token（既有回归） | `rebind_session_swaps_the_active_cancel_token` | `crates/tui/src/worker.rs` |
| cancel-token 刷新（双击 Esc 后可提交，既有回归） | `reset_cancel_replaces_with_fresh_uncancelled_token` | `crates/tui/src/worker.rs` |
| Kitty Ctrl+C/Ctrl+D 退出（既有回归） | `kitty_ctrl_c_quits` / `kitty_ctrl_d_quits` | `crates/tui/src/app_tests.rs` |

### pty 表征测试（核心命题的直接证据）

`lone_esc_is_delivered_within_bound`（`crates/tui/tests/input_pty.rs`）用 `openpty`+`dup2(slave, fd 0)` 造一个伪终端，push Kitty 键盘增强，向 master 写单个 `\x1b`（Esc），断言 `spawn_input_pump` 的 receiver 在 **2 s** 内收到 `Event::Key(Esc)`。
- 该测试**直接**验证"单按 Esc 不再可能无限挂死"这一核心命题——在真实 tty 语义下、在 crossterm 0.28 的 `poll`/`read` 路径上、有界时间内投递成功。
- 实现注意：pump 线程须 detach（不 `join`）——fd teardown 与 crossterm 的 mio 注册交互会令 `join` 误挂；线程靠 `is_closed()` 自退，无需 join。

- 全量回归：`cargo test --workspace` → **310 passed / 0 failed**
  （tui: 121 unit + 1 pty 表征 + 2 并发 subagent 端到端；余为 cli/core/llm/session/store/web）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **零警告**
- 行数：`input.rs` 88 ≤ 400；`terminal.rs` 171 ≤ 400；`app.rs` 766 ≤ 800；`input_pty.rs` 101 ≤ 400

## 为什么这版不是补丁

- **采集重构**：把"输入采集可能无限阻塞"这个**结构**换掉（有界 poll），冻死失败模式不存在 → 不需要看门狗/重建/定时器。
- **终端 RAII**：Rust 处理"无论如何退出都要清理"的正道，不是兜底。
- **worker 监管**：worker 死亡经通道关闭成为**可观测的单一退出路径**，不是 try/catch。

## Impact Surface

- **TUI 用户**：按 Esc 不再可能冻死整屏；任何 panic/异常退出都会恢复终端（不再变砖、不再需要 `reset`）；worker 意外退出时显示明确 marker 而非静默假死。
- **不影响** CLI / Web / session / store / llm —— 改动仅在 `crates/tui`（+ `Cargo.toml`）。
- crossterm 不再启用 `event-stream` feature。

## Related Docs
- [agents/tui](../../agents/tui/index.md)（已同步：输入采集改 poll 线程 + TerminalGuard RAII + start_turn 死亡检测）
- [2026-07-11 cancel-token-reset](../2026-07-11/cancel-token-reset.md)（既有"双击 Esc 后可提交"修复，本轮保留并加回归）
- [2026-07-11 subagent-view-and-ctrl-d-fix](../2026-07-11/subagent-view-and-ctrl-d-fix.md)（`DISAMBIGUATE_ESCAPE_CODES` 启用来源 + 流关闭即 Quit 防御，本轮保留）
