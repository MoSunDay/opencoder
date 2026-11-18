# 远端 `>` 强插补齐 + 退出不再可挂起（TUI）

一句话：队列面板 `>`（steer 立即提交）在远端会话不再静默无效 —— 中断以
`UiCmd::Interrupt` 命令发往 worker，由远端 worker 转成 Server `interrupt`；
同时 Ctrl+C/Ctrl+D 退出改为"本地永远取消令牌 + 非阻塞 `try_send` 交接"，
消除两类退出挂起。

## 背景与结论

### 1. 远端 `>` 静默无效

旧行为：`>` 点击走 `steer_fire::handle_steer_submit`，对"父级运行中且有
待吸收 steer"解析为 `SteerParent` 并触发**本地** `turn_cancel` 令牌。远端
会话（`chat.remote == true`）的 worker（`remote/worker.rs::spawn`）select
循环从不观察该令牌（只观察 `UiCmd::ResetCancel` 重绑的硬 `cancel` 令牌与
`commands.recv()`），Server 侧 turn 不会被中断，steer 无法在边界被强制吸收
—— 点击看起来毫无反应。

新行为：`steer_submit_after_mouse` 在 `chat.remote` 且结果为 `SteerOnly`、
且无存活 subagent 焦点时，向 worker `try_send(UiCmd::Interrupt)`。本地
worker 将其视为 no-op（本地 `>` 仍走共享 `turn_cancel`，可被回合中途观察）；
远端 worker 将其转为 `POST /commands {"action":"interrupt"}`（与既有
`cancel.cancelled()` 分支同体），Server runner 在边界断点处认领挂起的
steer。非阻塞发送：通道异常不得冻结点击路径。

### 2. 退出可挂起

旧行为（两处叠加）：

1. `handle_quit` 只在 `busy` 时取消会话令牌 —— `running`/`subagents_running`
   读到陈旧空闲而 worker 仍在 `run_session` 内时，`UiCmd::Quit` 排在整个
   turn 之后；
2. `cmd_tx.send(UiCmd::Quit).await` 是阻塞发送 —— worker 卡住且 64 容量
   通道占满时，UI 主循环在置 `quitting` **之前**就阻塞，终端冻结在
   alt-screen。

新行为：本地会话（`!chat.remote`）**无条件**取消共享令牌（陈旧空闲也中断
在途 turn）；远端保持豁免 —— 远端 detach 不得中断 Server 侧执行（Server
拥有运行权，语义不变）。Quit 交接改为 `try_send`：`Full`/`Closed` 时静默
丢弃，`app_bootstrap::finish` 随后 drop 发送端，两个 worker 循环都在通道
关闭（`None`）时退出，退出仍必然完成，而 UI 循环永不阻塞。

附带：`app.rs` 的 TranscriptReset 重建完成分支安装新视图时清空
`subagent_focus`（回放重推导块索引；陈旧索引会经
`is_live_subagent_focus` 误路由 `>`）。

## 变更清单

- `crates/tui/src/worker.rs`
  - `UiCmd` 新增 `Interrupt` 变体：远端 `>` 强插的 worker 侧载体；本地
    `process_cmd` 中为 no-op（返回 `false`）。
- `crates/tui/src/remote/worker.rs`
  - `commands.recv()` 匹配新增 `Some(UiCmd::Interrupt)` 分支（位于 `Some(_)`
    兜底之前）：`remote.created` 时向 Server 发
    `interrupt` 命令，失败走既有 `report` 上报路径。
- `crates/tui/src/app_loop_actions.rs`
  - `steer_submit_after_mouse`：`SteerOnly` 结局且远端且无存活 subagent
    焦点时 `try_send(UiCmd::Interrupt)`；`StartTurn` 路径不变。
- `crates/tui/src/app_loop.rs`
  - `handle_quit` 重写：本地永远取消令牌（`busy` 仅控制 "[exiting…]" 标记）；
    远端豁免保留；`send().await` → `try_send`，文档注释同步更新契约。
- `crates/tui/src/app.rs`
  - `maybe_reset` 安装重建视图的单行分支补 `subagent_focus = None;`
    （文件恰 800 行，保持单行）。

## 功能 → 测试名 → 文件

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 远端 `>` 点击发 Interrupt（worker 命令面） | `remote_running_steer_click_sends_interrupt` | `crates/tui/src/app_loop_tests/steer_remote_interrupt_tests.rs` |
| 本地 `>` 契约不回归（不发命令、本地令牌触发） | `local_running_steer_click_sends_no_commands` | 同上 |
| 存活 subagent 焦点不误伤远端父级 turn | `remote_subagent_focus_click_does_not_send_interrupt` | 同上 |
| Interrupt 到达 Server（远端 worker 集成） | `remote_interrupt_command_targets_server` | `crates/tui/src/remote/tests/mod.rs` |
| 本地空闲退出仍取消令牌（新行为） | `quit_cancels_local_token_even_when_idle` | `crates/tui/src/app_loop_tests/quit_unblock_tests.rs` |
| 本地忙退出取消令牌（既有契约锁定） | `quit_cancels_local_token_when_busy` | 同上 |
| 远端 detach 豁免（令牌不取消、Quit 仍投递） | `quit_does_not_cancel_remote_token` | 同上 |
| 满通道下退出永不阻塞（try_send 丢弃于 Full） | `quit_never_blocks_on_a_full_channel` | 同上 |

## 实测

定向测试（实际执行）：

```text
cargo test -p opencoder-tui --lib quit_unblock          # 4 passed / 0 failed
cargo test -p opencoder-tui --lib steer_remote_interrupt # 3 passed / 0 failed
cargo test -p opencoder-tui --lib remote_               # 17 passed / 0 failed
cargo test -p opencoder-tui --lib steer                 # 67 passed / 0 failed
```

- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` →
  通过（exit 0，零警告）。
- 行数上限：`app.rs` 800、`app_loop.rs` 799、`app_loop_actions.rs` 797、
  `remote/worker.rs` 237、`remote/tests/mod.rs` 411（迭代文件 ≤800）；
  `quit_unblock_tests.rs` 99、`steer_remote_interrupt_tests.rs` 161
  （新增文件 ≤400）。
- **未运行全量测试**：仅运行上述 quit/steer/远端相关定向用例。
