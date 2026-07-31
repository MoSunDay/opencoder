Commit: (working-tree, pre-initial-commit)

# fix(tui): `/task` 切换时 cancel 先于 Quit，中断旧 worker 飞行中的 turn

## 背景

`/task` 切换会话时 `switch_session` 直接向旧 worker 发 `UiCmd::Quit`。但
worker 主循环是严格顺序的 `recv -> process_cmd -> recv`：如果切换发生时旧
worker 正在执行一个 turn（LLM 流式输出 / 工具批量执行中），`Quit` 被排入
channel 后不可见——必须等该 turn 自然结束后 worker 才回到 `recv()` 看到
`Quit`。对长流式响应（数百 chunk）这会造成明显的 UI 冻结，切换看上去卡死。

## 变更

### `crates/tui/src/app_task.rs`（`switch_session`）
- 在 `cmd_tx.send(UiCmd::Quit)` **之前**插入 `cancel.cancel()`。
- cancel 是 loop 当前绑定的活动 token（`rebind_session` 每个 session 重指），
  取消它会中断 LLM 流（`llm_call.rs` 的 `select!` 臂监听 `await_cancel`）和
  工具批量执行（`execute.rs` 的 `select!` 臂），使 turn 快速返回；
  `run_loop` 在循环顶部 `is_cancelled()` 检查处 break，worker 回到 `recv()`
  观察 `Quit` 并退出。
- cancel 同时传播到 subagent（子 token 派生自父 token），经
  `cancel_subagent_task` 将其任务标记 `Cancelled`——`resume_and_replay`
  只 replay `Running` 任务，故切回时不再阻塞。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| cancel-token 交换（每个 session 重指活动 token） | `rebind_session_swaps_the_active_cancel_token` | `worker.rs` (inline `mod tests`) |
| cancel-token 刷新（双击 Esc 后可提交） | `reset_cancel_replaces_with_fresh_uncancelled_token` | `worker.rs` (inline `mod tests`) |

> 注：cancel-before-Quit 的端到端回归测试尚未添加（需要模拟长流式
> turn + 验证 worker 在超时内退出）。当前覆盖来自 cancel 基础设施的
> 既有单元测试。

## Impact Surface
- 用户可感知：`/task` 切换不再卡死等待飞行中 turn 完成。
- 不影响：Store/ChatStream/session runner 语义；`switch_session` 签名不变。
