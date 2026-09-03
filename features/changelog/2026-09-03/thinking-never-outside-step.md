# thinking 永不出现在 step 阶梯之外（TUI + SPA 双端收敛）

Commit: af71944

## 背景

9b999cf 落地 step 三级下钻时保留了旧取舍：「纯文本轮（无任何工具调用）保留独立 Thinking 块」。用户反馈 TUI 仍在 step 之外渲染 thinking，要求统一语义：**thinking 永不出现在 step 阶梯之外**——纯文本轮折叠为「零调用 step」（只有 thinking、无聚合行），与工具轮一起共享同一座阶梯。

## 泄漏根因（4 类场景）

1. 纯文本轮：无任何工具调用，旧逻辑整轮 thinking 留在顶层。
2. 工具轮的收尾轮：thinking + Say 结束（后面没有更多 tool call），旧逻辑 flush 成独立 Thinking。
3. steer 边界前：边界阻止吸收，thinking 滞留顶层。
4. task-only 轮：`ToolStart` 对 `name=="task"` 提前 return 不吸收，thinking 滞留。

## 变更

### TUI（`crates/tui/src/`）

- `chat_steps.rs`：新增共用落位核心 `place_thinking_step(blocks, thinking) -> Option<usize>`——先 `absorb_pending_thinking`（walk-back 收集段内全部透明 Assistant/trailing think），再落位：尾随 `StepGroup` 直接追加零调用 step（`calls: vec![]`）；否则 walk-back 定位——Assistant 透明（记录 insert_at），撞到其余边界块时插到边界**之后**（`insert_at = i + 1`，梯子归属当前 user 段，绝不越过 prompt）；`thinking_step_group` 构造零调用组。`flush_pending_thinking = place(absorb(...))`。模块 doc 同步改写。
- live 接线 7 处：`chat.rs`（Done/Error finalize 后、SteerConsumed User 回显前、SubagentStart finalize 后且在 `hidden_assistant_idx` 计算前、push_marker/push_marker_lines）、`chat_helpers.rs`（push_bash_tool）、`compaction_block.rs`（open_compaction_streaming）；`chat_stream.rs` 封装 `ChatView::flush_pending_thinking`（含 `hidden_assistant_idx >= insert_at` 时 +1 簿记）。
- replay：`coalesce_steps` 同源改写，Assistant 臂透传 pending、`other` 边界臂与函数尾部、零步防御分支统一走 `place_thinking_step`——旧格式转录回放同样折叠。
- 渲染/核算/点击天然支持零调用 step（无聚合行、仅 thinking），无需改动。

### SPA（`crates/web/spa/src/reduce.js`）

- `placeThinkingStep(turns, thinking)` + `flushPendingThink(turns)`（copy 后 `absorbSegmentThinking` 收集再落位），与 TUI 同规则；边界落位同样 `i + 1`。
- 接点：live `done`/`error` 帧、`queue_consumed`/`steer_consumed` echo、tool_start task 分支、`subagent_start`、`status`/`compaction`/`agent_switched`/`model_switched` sys turn 推入（对齐 TUI push_marker 语义）、`withUserTurn`（chat.jsx 乐观回显）；快照路径 text 分支（`!toolsFollow`）与 walk 尾部。

## 测试清单（规则 01/02：每行为有测试）

- `chat_tests/step_group.rs`：新增 `pure_text_turn_folds_thinking_into_a_call_less_step`、`tool_turn_final_round_folds_its_say_round_thinking`、`error_turn_folds_pending_thinking_into_the_ladder`、`user_echo_flushes_pre_boundary_thinking_into_the_ladder`、`mid_conversation_flush_lands_after_the_user_prompt`（中途对话：梯子必须落在 prompt 之后而非之上）、`replay_folds_thinking_without_a_following_group_into_the_ladder`；2 例旧语义锁定重写为折叠语义。
- 夹具修复：`ctrl_l_tests.rs`（展开循环加 `!steps[0].calls.is_empty()` 守卫）、`mouse_tests.rs`、`render_tests/mod.rs`（thinking_view）、`thinking_state.rs` 三处 mid-stream 夹具去掉 Done；`completed_answer_creates_say...` 断言翻转为 StepGroup。
- `reduce.test.js`：(c3)/(c4)/(l)/(m)/dangling-flush 5 例翻转为 steps turn 形态。

## 回归

- `cargo test -p opencoder-tui`：**1709 passed / 0 failed**（净增 4 用例，含中途对话落位）。
- `cargo test --workspace`：**3965 passed / 0 failed**。
- `cd crates/web/spa && npx vitest run`：**141 passed / 0 failed**。
- `scripts/build-spa.sh` 已重建 + `scripts/check-spa-drift.sh`：**no drift**。
- 真机 smoke（`scripts/browser-acceptance.js`/`scripts/e2e_glm.py`，需真实 LLM 凭证）离线不可行，维持可选。
