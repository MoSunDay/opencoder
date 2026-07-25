Commit: (working-tree, pre-initial-commit)

# TUI「闲置后整屏冻死、Ctrl+C/D 无响应」根因修复（活性监管 supervisor + 心跳）

## 背景

用户报告：**opencode 闲置一段时间后整屏彻底无响应——连 Ctrl+C / Ctrl+D 都退不出，必须 kill 进程**。
触发场景：SSH 断连、tmux pane 被关、终端窗口被关闭——**发生在进程完全空闲、没有任何交互时**。

前一轮（`2026-07-12/tui-freeze-rootcause`）已把输入采集从 async `EventStream`
重构为同步 `poll`/`read` 线程，修复了「按 Esc 后冻死」。但本轮确认该重构**未覆盖
tty 自身死亡**这一失败向量。

## 诊断

### 根因：crossterm 0.28 的 mio `UnixInternalEventSource::try_read` 在 tty 死亡时永久忙循环

当 tty 返回 `Ok(0)`（EOF）或 `EIO` 时——**正是 SSH 断连 / tmux pane kill / 终端关闭时发生的情况**——
crossterm 0.28 的 mio 事件源 `try_read` 进入**永久忙循环**（既不返回事件，也不返回错误，
永远 spin）。该忙循环持有 crossterm 的全局 `INTERNAL_EVENT_READER` 互斥锁（`parking_lot::Mutex`）。

后果链：
1. 输入泵线程的 `event::poll(150ms)` **永远不返回**（拿不到全局锁）。
2. 心跳停止推进，没有任何按键能到达主循环。
3. 整屏冻死，进程活着但完全无响应——Ctrl+C / Ctrl+D 也无效（信号处理器与终端
   restore 都依赖正常代码路径执行）。

源码位置：`crossterm-0.28.1/src/event/source/unix/mio.rs::try_read`。

### 排除的其它向量

- **session runner**：`run_loop` 有 doom-loop 守卫（`DOOM_THRESHOLD=3`）、steer/queue
  提升、120s 流空闲超时——非冻死原因。
- **LLM 客户端**：120s stream idle timeout + 300s read timeout——非冻死原因。
- **store/db**：libsql WAL + `busy_timeout`——非冻死原因。

这些层此前已充分防护；本轮冻死**仅在 tty 死亡时**经由 crossterm mio 忙循环触发。

## 变更

### 活性监管 supervisor（核心）

在**专用 OS 线程**（不依赖 tokio、不受互斥锁死锁影响）上运行一个看门狗：

- **`crates/tui/src/supervisor.rs`**（新增，220 行）：
  - `Heartbeat`：`Arc<AtomicU64>`（毫秒级 epoch 时间戳）。输入泵在**每次阻塞 poll 之前** bump。
  - `trip_reason()`：纯决策函数，判定是否应触发退出：
    - 心跳在 `WEDGE_TIMEOUT = 5s` 内未推进 **且** app 仍 active → `Wedge`。
    - 收到 `SIGHUP` / `SIGINT` / `SIGQUIT` / `SIGTERM` → `Signal`（无论 active/staleness）。
    - 正常关闭（`supervisor_active = false`）→ 忽略 staleness。
  - `spawn()`：启动 OS 线程，每 `POLL_INTERVAL = 1s` 轮询一次；触发时调用
    `TerminalGuard::restore()`（幂等：恢复终端 raw 模式 → alternate screen 退出）后 `exit(0)`。
  - 信号监听经 `signal-hook`（已在 `Cargo.lock` 中，crossterm 传递依赖）。

### 心跳接入

- **`crates/tui/src/input.rs`**：`spawn_input_pump` 接收 `Heartbeat`，在每次
  `event::poll` 之前 `heartbeat.bump()`。若 poll 因 mio 忙循环永不返回，bump 停止，
  supervisor 在 5s 后检测到 stall 并恢复终端、干净退出。

### 生命周期接线

- **`crates/tui/src/app.rs`**：
  - `run_app` 启动时 `supervisor::spawn(heartbeat, supervisor_active)`。
  - 正常关闭路径：在 drop 输入泵**之前** `supervisor_active.store(false)` disarm supervisor，
    避免正常退出被误判为 stall。

### 依赖

- **`crates/tui/Cargo.toml`**：新增 `signal-hook = "0.3"`（锁定版本与 lockfile 一致）。

### 附带修复（预存编译错误，非本任务引入但阻塞全量回归）

- **`crates/tui/src/app_loop_tests.rs`**：
  - 修正 `use crate::app_loop::env_model_override` → `crate::app::app_loop::env_model_override`
   （`app_loop` 是 `app` 的子模块，声明于 `app.rs:32`）。
  - 移除 `fold_stale_turndone_keeps_newer_turn_running` 测试中已失效的 `pending_handoff`
    参数（该参数在 `2026-07-24` handoff 重构中已从 `fold_ui_events` 签名移除，但测试调用点未同步）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| trip_reason：fresh + quiet → 不触发 | `trip_reason_no_trip_when_fresh_and_quiet` | `supervisor.rs` |
| trip_reason：stale + active → Wedge | `trip_reason_wedge_when_stale_and_active` | `supervisor.rs` |
| trip_reason：shutdown 期间忽略 staleness | `trip_reason_ignores_staleness_during_shutdown` | `supervisor.rs` |
| trip_reason：信号优先于 active/staleness | `trip_reason_signal_wins_regardless_of_active_or_staleness` | `supervisor.rs` |
| trip_reason：边界严格大于 | `trip_reason_boundary_is_strictly_greater` | `supervisor.rs` |
| Heartbeat：bump 推进时间戳 | `heartbeat_advances_on_bump` | `supervisor.rs` |
| tty 死亡忙循环表征（EOF） | `input_pty` | `tests/input_pty.rs` |
| tty 死亡忙循环表征（不完整读） | `input_pty_incomplete` | `tests/input_pty_incomplete.rs` |

- 全量回归：`cargo test -p opencoder-tui --lib -- --test-threads=1` → **413 passed; 0 failed**。
  （注：`apply_skill_tokens_resolves_and_activates_known_skill` 在并行模式下因 HOME 环境变量
  竞争偶发失败——预存 flake，commit `ac44ad2` 已尝试用 HOME mutex 修复但仍残留；单跑 / 串行均通过，
  与本轮改动无关。）
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告。
- 行数：`supervisor.rs` 220 ≤ 400；`input.rs` ≤ 400；`app.rs` ≤ 800。

## 为什么这版不是补丁

- **结构层面而非兜底**：不在冻死后尝试「检测并恢复」（那需要被冻死的线程自己醒来——不可能）。
  而是用一个**完全独立于冻死路径**的 OS 线程，从外部观测「输入泵是否还在推进」。
- **不依赖 tokio / 互斥锁**：supervisor 线程只读一个 `AtomicU64` + 一个 `AtomicBool`，
  无锁、无 async、不受 crossterm 全局互斥锁死锁影响——这正是冻死时唯一仍能运行的代码。
- **信号兜底**：tty 死亡时进程常收到 `SIGHUP`；即使心跳未 stale，信号也能立即触发干净退出。
- **正常路径零开销**：仅多一个每秒读两个 atomics 的线程；正常退出时 disarm，绝不误杀。

## Impact Surface

- **TUI 用户**：SSH 断连 / tmux kill / 关终端后，进程不再僵死挂起——supervisor 在 5s 内
  恢复终端（raw mode 回落、alternate screen 退出）并干净退出，不再留下冻死进程需要手动 kill。
  收到 SIGHUP/SIGINT/SIGQUIT/SIGTERM 时同样保证终端恢复。
- **不影响** CLI / Web / session / store / llm —— 改动仅在 `crates/tui`（+ `Cargo.toml`）。
- 无配置项、无行为开关；supervisor 始终随 TUI 启动。

## Related Docs
- [2026-07-12 tui-freeze-rootcause](../2026-07-12/tui-freeze-rootcause.md)（前一轮：async EventStream → 同步 poll 线程 + TerminalGuard RAII；本轮补上 tty 死亡向量）
- [agents/tui](../../agents/tui/index.md)
