Commit: f0ccec4c76f583607f43022179c2c5c745f7a3ef

# TUI 手动压缩成功路径补发终端 Done 帧（web parity）

## 背景

- web 侧 `handle.rs DrainCmd::Compact` 对每个 `Ok(_)` 结果都会以 `SessionEvent::Done` 收尾；TUI worker 的 `UiCmd::Compact` 此前只在 `Err` 时发 Error、成功时只发 `TranscriptReset`/`Compaction`/`TurnDone`，不发 Done。
- app_loop 的 Done 处理器会从 store 重新同步 pending Queue/Steer 行并 arm `drain_pending`；缺 Done 时，压缩 turn 期间被接纳的输入会永久滞留在 store（`TurnDone` 不会触发 resync）。

## 变更

- `crates/tui/src/worker.rs` `UiCmd::Compact` 的 `match outcome`：
  - `Ok(Some(summary))`：在 `TranscriptReset` + `Compaction` 之后追加 `SessionEvent::Done`（`sink.push` 持久化 + `forward_event` 实时转发，与其他帧一致）。
  - `Ok(None)`（"还没有可压缩内容"）：同样补发 Done——web 对每个 `Ok(_)` 都发 Done，两个前端的 idle 边界保持一致。
  - `Err`：保持不变，仅发 Error，不补发 Done（失败不视为完成的 drain 命令，避免自动重启误判造成错误循环）。
- 帧顺序：`TranscriptReset` → `Compaction` → `Done`（成功有摘要）/ `Done`（无摘要）；随后照旧 `drop(sink)` → flusher 收尾 → `TurnDone`。

## Validation

- `cargo check -p opencoder-tui`：通过（dev profile，无错误/警告）。
- 行为对齐参照：`crates/web` handle.rs DrainCmd::Compact 的 Ok/Err 分支发帧策略。
