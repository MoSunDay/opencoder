Commit: (working-tree, 基于 b44dfc8)

# 每个用户输入一个阶梯：Turn = n Steps + Say

## Context

Turn 契约：1 个 Turn = n 个 Step + Say；Step = 一段 Thinking + n 个 function call。默认（收起）视图每个用户输入渲染一对 `[▸ N Steps] + [Say]`。TUI 存在两处违背：

1. **Steer 吸收不换梯**：`turn_block_start` 只在 `begin_turn()`（提交路径）重置；run 中途被吸收的 steer 虽在 `SteerConsumed` 回显了 User 块，但其后 rounds 的 Step 仍并入回显**之前**的旧 StepGroup——所有 Turn 的 Step 堆进一个阶梯，回显夹在阶梯与 Say 之间。
2. **Queue 消费错位**：idle 边界的 drain 重启先 `begin_turn()`（floor=当时 blocks.len()），QueueConsumed 回显**之后**才落地；`merge_turn_call` 在 stale floor 插入新组，`▸ N Steps` 渲染在被消费 prompt 回显**上方**。

SPA 无此缺陷（`steps/reducer.js::lastUserBoundary` 以 `steer_consumed`/`queue_consumed` 帧为硬边界），本次把 TUI 对齐到同一契约。

## Change Summary

- `ChatView::reanchor_turn_after_user_echo()`（crates/tui/src/chat.rs）：冻结旧阶梯 `progress_active`（该 Turn 已无自己的 Say）并把 `turn_block_start` 重锚到回显块之下。
- `SteerConsumed` 回显落地处（chat.rs）与 `QueueConsumed` 回显落地处（app_loop.rs）在推块后调用重锚；bare control（空回显）不重锚、不拆梯。
- 语义文档同步：`chat_steps.rs`/`chat_types.rs` 模块注释改为“每个用户输入（提交/steer 消费/queue 消费）锚定一个新阶梯”。

## Verification

- `cargo test --workspace`：128 个测试目标全部 ok、0 failed（含新增 6 例）。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cd crates/web/spa && npm test -- --run`：151/151（SPA 无代码变更，回归确认）。
- `scripts/check-spa-drift.sh`：no drift。

### 测试清单

| 保证 | 测试 |
| --- | --- |
| steer 消费开启新阶梯且位于回显之下、Say 尾随 | `chat_tests/step_group/turn_boundary.rs::steer_consumed_starts_a_new_ladder_below_the_echo` |
| steer 边界冻结旧阶梯动效 | `turn_boundary.rs::steer_boundary_freezes_the_previous_ladder_progress` |
| bare control steer 不拆梯 | `turn_boundary.rs::bare_control_steer_keeps_one_ladder` |
| queue 消费阶梯渲染于其回显之下 | `turn_boundary.rs::queue_consumed_ladder_renders_below_its_prompt_echo` |
| 连续提交各自渲染 `N Steps`+Say 顺序契约 | `turn_boundary.rs::consecutive_turns_render_steps_then_say_each` |
| app 层 QueueConsumed 重锚（floor 移至回显下、后续组位于回显下） | `app_loop_tests/done_error_mirror_tests.rs::fold_queue_consumed_reanchors_ladder_below_echo` |
| 提交型边界不变量（更名） | `step_group.rs::user_inputs_are_the_only_live_step_group_boundaries` |

## Related Docs

- [Thinking 定义 Step 边界](thinking-defined-step-boundary.md)
- [tui 模块](../../../agents/tui/index.md)
