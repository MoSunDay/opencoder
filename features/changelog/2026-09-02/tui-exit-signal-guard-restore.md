# TUI 退出终端状态恢复——信号守卫前移至 TerminalGuard::enter

Commit: (working-tree, 终端退出恢复状态收敛)

## 背景

- 用户反馈：TUI 退出后终端「滑动/点击输出特殊字符」——即进程死亡时鼠标上报（`?1000h` 系）未关闭，宿主 shell 把鼠标转义序列当输入回显。
- 原有恢复面已有三层：`TerminalGuard` RAII Drop、panic hook、liveness supervisor 的信号臂（SIGHUP/SIGINT/SIGQUIT/SIGTERM → restore + exit）。
- 缺口：supervisor 在 `run_app`/onboarding 各自 spawn，`TerminalGuard::enter()`（开鼠标捕获）到 supervisor 就绪之间存在启动窗口——该窗口内 SIGTERM/SIGHUP（tmux kill-pane、SSH 掉线）按默认处置裸杀进程，恢复序列一行都不会发出。
- 实测验证（pty 抓字节）：正常退出、主循环/onboarding 阶段信号退出恢复序列齐全；缺口仅在进入捕获后的极早窗口，结构性存在。

## 变更

- 新增 `crates/tui/src/signal_guard.rs`：进程级单例信号守卫，`TerminalGuard::enter()` 捕获终端的第一毫秒即武装（latch `AtomicBool` CAS 保证只 spawn 一次）；watcher 线程 250ms 轮询 SIGHUP/SIGINT/SIGQUIT/SIGTERM，触发即 `TerminalGuard::restore()` → stderr 说明（含信号名 + `--continue` 提示）→ `exit(0)`。
- `crates/tui/src/terminal.rs::enter`：捕获成功后 `signal_guard::arm_once()`——覆盖 onboarding 向导、chat 主循环、关机窗口等全部路径。
- `crates/tui/src/supervisor.rs` 收敛为纯「输入泵心跳楔死」看门狗：删除信号注册与 `Trip::Signal`（`trip_reason` → `is_wedged` 布尔判定），避免两个线程竞态双写恢复序列（交错写入可能把同一段转义序列撕成垃圾、个别 disable 失效）。信号职责移交 signal_guard（单一写者）。
- 语义不变项：wedge 超时 5s、`active=false` 停机期不误报、退出前先 restore 再打 stderr（alt-screen 内 stderr 是乱码）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 守卫 latch 恰好武装一次（单写者保证） | `try_arm_latches_once` | crates/tui/src/signal_guard.rs |
| 信号退出文案（二进制名/信号名/resume 提示/无裸 opencode 残留） | `exit_message_names_signal_and_resume_hint` | crates/tui/src/signal_guard.rs |
| 楔死判定阈值/停机豁免/严格大于边界 | `is_wedged_{false_when_fresh_and_quiet,true_when_stale_and_active,ignores_staleness_during_shutdown,boundary_is_strictly_greater}` | crates/tui/src/supervisor.rs |
| 楔死退出文案 | `exit_message_leads_with_opencoder_and_resume_hint` | crates/tui/src/supervisor.rs |
| 恢复负载序列完备（pop-kitty/鼠标/粘贴/alt-screen） | `write_restore_emits_all_restoration_sequences` | crates/tui/src/terminal.rs（已有，回归） |
| e2e：捕获后 SIGTERM 必发恢复序列 + 用户说明 | `sigterm_after_capture_restores_terminal` | tests/tui_exit_restore_e2e.rs |
| e2e：Ctrl+D 正常退出必发恢复序列 | `normal_quit_restores_terminal` | tests/tui_exit_restore_e2e.rs |
