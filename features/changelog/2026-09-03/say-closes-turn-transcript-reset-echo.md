Commit: (working-tree, 基于 5410f6d)

# Say 收合 Turn：阶梯落于 Say 之下；TranscriptReset 保留在途回显

## Context

Turn 契约 `1 Turn = n Steps + Say` 存在两处漏洞：

1. **回合间不收合**：一次提交可含多个回合（agent 在工具阶段之间开口说 Say）。旧
   `append_text_delta` 只把 chunk 拼进开着的 Say 或开新 Say 块，从不前移
   `turn_block_start`——Say 之后的 reasoning/calls 仍落进同一个 StepGroup，且被收合
   回合的阶梯要等 `LlmRoundEnd` 才 seal（token 记账滞后、replay 重建顺序也可能翻转）。
2. **TranscriptReset 抹掉在途回显**：复合控制命令 `/act_clear_context <tail>` 的
   提交路径先本地回显 tail 再起 run；runner 应用 ClearContext 后 `TranscriptReset`
   从折叠 transcript 重建视图——tail 此时尚未落库，重建把运行中 Turn 的 User 边界
   整个抹掉：新回合的阶梯读起来并入上一个 Turn、Say 与旧 Say 粘连。

## Change Summary（crates/tui）

- `chat_stream.rs::append_text_delta`：新 Say 开块 = 关闭当前 Turn——先
  `flush_pending_thinking`（Say 前的 pending thinking 折入被收合回合的阶梯，无 call
  则成 call-less step）再 `seal_trailing_step`（收合回合的 thinking 当场计入
  `context_used` 恰一次），随后把 turn floor 前移到 Say 之下：下一回合的阶梯落在
  Say 之后。`seal_trailing_step`/`reconcile_completed_assistant` 由 floor-first 改
  `rposition`（Say 前移 floor 后，可能持有未 seal 尾 step 的阶梯永远是流上最后一个
  group；被 reconcile 的 Say 是流上最后一个 Assistant）。
- `chat_steps.rs::absorb_pending_thinking`：sealed Thinking 不再算 pending（已在
  收口处计数渲染），遇 sealed 即停。
- `chat_types.rs`：`ChatView::pending_turn_echo` ——在途回合用户回显的字面记忆；
  `chat.rs`（SteerConsumed 回显）、`app_helpers.rs::push_user`（提交路径）、
  `app_loop.rs`（QueueConsumed 回显）落地时记录，`Done`/`Error` 退休（bare 控制
  命令空回显不动它）。
- `session_ui/replay.rs`：replay 按 BLOCK ORDER 重建（Text 块关回合，镜像 live
  契约，resume 不再翻转 `Thinking -> Say`）；`rebuild_after_reset` 保留并在重建块
  之下重推 `pending_turn_echo` 的 User 块 + marker，随后重锚 `turn_block_start`——
  运行中 Turn 的用户边界跨 reset 存活。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| TranscriptReset 后在途回显回到重建视图之下、阶梯锚定其下（回归主例） | `app_loop_tests/transcript_reset_echo_tests.rs::transcript_reset_restores_in_flight_echo_below_rebuilt_view` |
| Done 清 echo，之后的 bare reset 不复活旧 prompt | 同文件 `done_clears_pending_echo_so_later_bare_reset_does_not_resurrect_it` |
| SteerConsumed 回显被记忆、Done 退休 | `chat_tests/step_group/turn_boundary.rs::steer_consumed_echo_is_remembered_for_reset_restore` |
| bare 控制命令（空回显）不动在途 echo | 同文件 `bare_control_consumed_leaves_pending_echo_untouched` |
| Say 收口处 step thinking 计入 context_used 恰一次 | `chat_tests/mod.rs::ctx_counts_reasoning_once_at_finalize`（语义更新） |
| 回合间新阶梯落在 Say 之下、折叠 raw 分段正确 | `chat_tests/thinking_state.rs`（`collapsed_live_reasoning_stays_raw_until_the_step_opens` 等随新语义更新） |

## 回归

- `cargo test -p opencoder-tui --lib`：全绿（含新增 4 例 + 更新 3 例）。
- 按用户指示本轮跳过 clippy/test（fmt 已过）；下一轮迭代前需补全量回归。
- SPA 无代码变更（其 reducer 本就以 Text 块推进回合边界，无此缺陷）。

## Related Docs

- [每个用户输入一个阶梯](turn-boundary-per-user-input.md)（本篇把契约延伸到"回合间 Say"与 reset 重建两个缺口）
- [tui 模块](../../../agents/tui/index.md)
