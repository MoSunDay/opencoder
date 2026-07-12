Commit: (working-tree, pre-initial-commit)

# TUI「按 Esc 后整屏冻死、必须 kill 进程」根因修复（输入采集重构 + 终端 RAII + worker 死亡监管）

## 背景

用户报告：**按 Esc（中止 / 返回）有概率导致整屏彻底无响应——连 Ctrl+C / Ctrl+D 都退不出，必须 kill 进程**。

## 诊断

### 根因：主循环活性被绑在 crossterm `EventStream`（async/mio）上，该 reader 任务可停滞

- Esc 处理逻辑本身（`app.rs::KeyAction::Cancel`）**全是同步代码**，不可能阻塞事件循环——排除"Esc 逻辑冻死"。
- `run_app` 用 `crossterm::EventStream`（`app.rs` 旧 `events.next()`）。该 async stream 的内部 reader 任务走 mio + tokio waker；一旦该任务停滞（不再 resolve），`tokio::select!` 的输入分支永不触发。
- **机制澄清（源码级）**：同步 `event::poll`/`read` 路径本身**不会**被单按 Esc 挂死——crossterm 0.28 unix source 用 `filedescriptor::poll`（有界）+ 非阻塞读，且 `parse.rs:36-42` 对 `more=false` 的单个 `\x1b` 立即提交为 Esc。故冻死不在"解析器消歧阻塞"，而在 **EventStream 的 async/mio 层停滞**。修法的本质是**绕过该 async 层**，而非"用有界 poll 防消歧挂死"。
- 一旦停住：主循环不再 poll 任何事件 → raw 模式下 Ctrl+C/Ctrl+D 交给程序、程序不读就无人响应 → 进程还活着但整屏死 → 必须 kill。与用户描述吻合。

故根因是：**主循环的活性被绑在一个可停滞的 async EventStream 上**。正确修法不是"发现它卡了再重建"（那是不可持续的补丁），而是**改走同步有界 `poll`/`read` 路径，从结构上绕过该失败模式**。

### 顺带确认的其它冻死/卡死/变砖隐患（本轮一并修）

- **终端恢复缺失**：旧 `run()` 的清理（`disable_raw_mode` / `PopKeyboardEnhancementFlags` / `DisableMouseCapture` / `LeaveAlternateScreen`）只在 `run_app` 正常返回时跑；全仓无 `Drop`/`catch_unwind`/panic hook。任何 panic 都跳过清理 → 终端留在 raw+alt-screen+鼠标捕获 = 肉眼与"冻死"无法区分，需 kill + `reset`。
- **worker 静默死亡**：worker 是裸 `tokio::spawn`，panic 后 `cmd_rx` 关闭、不再发 `TurnDone` → `running` 永远 true、提交全进死通道，UI 看似"啥都不干"。

## 变更（根因级，无补丁/无定时器/无 try-catch 兜底）

### 1. 输入采集重构：专用 OS 线程 + 同步有界 `poll`/`read`，弃用 `EventStream`（核心）
- **`crates/tui/src/input.rs`（新增）**：`spawn_input_pump() -> (mpsc::Receiver<Event>, thread::JoinHandle)`。`std::thread::spawn` 跑循环：`event::poll(150ms)` 命中则 `event::read()`，经 tokio mpsc `blocking_send` 投递。
  - 本质是**绕过 EventStream 的 async/mio 层**，改走同步路径。该路径端到端有界：crossterm unix source 用 `filedescriptor::poll`（受 timeout 约束）+ 非阻塞读；单个 `\x1b`（`more=false`）立即提交为 Esc（`parse.rs:36-42`）；`poll` 成功后的 `read()` 直接从内部队列弹已入队事件，不触及其 `poll(None)` 回退路径。线程每 ≤150ms 必醒 → 冻死失败模式从结构上消失。
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
| **同步 pump 在 pty 上有界投递单按 Esc（基础交付回归）** | `lone_esc_is_delivered_within_bound` | `crates/tui/tests/input_pty.rs`（新增，pty 表征） |
| **不完整 CSI（`\x1b[`）不挂死 pump；补全字节有界投递（结构不变量：pump 不会卡在半个序列上）** | `incomplete_csi_does_not_wedge_pump` | `crates/tui/tests/input_pty_incomplete.rs`（新增，pty 表征） |
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

### pty 表征测试（两个互补的不变量）

两个测试各自独占一个集成测试二进制（进程），因为 `dup2(fd 0)` 是进程级全局。共用 `tests/common/mod.rs` 的 `PtyStdin` 线束（openpty + raw + fd 0 重定向 + Drop 恢复）。

- **`lone_esc_is_delivered_within_bound`**（`tests/input_pty.rs`）：向 master 写单个 `\x1b`，断言 2s 内收到 `Esc`。验证同步 pump 端到端在 pty 上有界投递——基础交付回归。**注意**：单个 `\x1b`（`more=false`）在同步路径上本就立即提交（`parse.rs:36-42`），故此测试不复现原始冻死（那是 async/mio 层停滞），而是回归守护"pump 能投递输入"。
- **`incomplete_csi_does_not_wedge_pump`**（`tests/input_pty_incomplete.rs`）：向 master 写 `\x1b[`（不完整 CSI，解析器缓冲并返回 `Ok(None)`，见 `parse.rs:140-141`），等 400ms（远超 150ms poll 窗口，确保 pump 已超时并至少空转一轮），再写 `A`（补全为 `\x1b[A` = Up），断言 2s 内收到 `Up`。**这是结构不变量的直接证据**：不完整序列不会挂死 pump 线程，补全字节到达后仍能投递。若 pump 在半个序列上卡住，补全字节永远不会被读，此测试超时失败。
- 实现注意：pump 线程须 detach（不 `join`）——fd teardown 与 crossterm 的 mio 注册交互会令 `join` 误挂；线程靠 `is_closed()` 自退，无需 join。

- 全量回归：`cargo test --workspace` → 全绿（tui: 121 unit + 2 pty 表征 + 2 并发 subagent 端到端；余为 cli/core/llm/session/store/web）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **零警告**
- 行数：`input.rs` 96 ≤ 400；`terminal.rs` 171 ≤ 400；`app.rs` 766 ≤ 800；`input_pty.rs` 54 / `input_pty_incomplete.rs` 63 / `common/mod.rs` 92 ≤ 400

## 为什么这版不是补丁

- **采集重构**：把"主循环活性绑在可停滞的 async EventStream 上"这个**结构**换掉——改走同步有界 `poll`/`read`，端到端绕过该失败模式 → 不需要看门狗/重建/定时器。
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
