Commit: (working-tree, pre-initial-commit)

# feat(session/tui/web): subagent steer — turn-level interrupt

## 背景

用户在 subagent 执行过程中无法插入方向修正（steer）。subagent 的 steer 语义与
主 session 不同：只允许 steer（不允许 queue），避免竞态；subagent 结束后不再接受
提交。关键设计：">"按钮触发 **turn-level interrupt**——打断当前 LLM/工具 turn，
但保持子 session 的 `run_loop` 运行；不改变 subagent 状态。

## 变更

### Session crate

- **`crates/session/src/lib.rs`**：`SessionState` 新增 `turn_cancel: Option<SharedCancel>`
  与 `child_turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>>`；`SharedCancel`
  类型别名 = `Arc<std::sync::Mutex<CancellationToken>>`（std Mutex，不跨 `.await` 持有）。
  `new()` 与 `resume.rs` 初始化。
- **`crates/session/src/runner/steer.rs`**：新增 `await_turn_cancel()`、
  `is_turn_cancelled()`、`reset_turn_cancel()` 辅助函数。
- **`crates/session/src/runner/llm_call.rs`**：`turn_cancel_fut` + `biased select!` 臂，
  触发时返回空 turn。
- **`crates/session/src/runner/execute.rs`**：subagent 与 leaf-tool 两个 `select!` 块
  均增加 turn_cancel 臂，触发时输出 "turn interrupted"。
- **`crates/session/src/runner/mod.rs`**：`run_loop` 中两处检查——LLM 调用后
  （reset+continue，跳过记录）与工具批次后（reset+continue，记录工具结果后）。
- **`crates/session/src/runner/subagent.rs`**：subagent 启动时在
  `parent.child_turn_cancels[call_id]` 注册子 turn-cancel token；结束时移除。

### Web API

- **`crates/web/src/handle.rs`**：`SessionHandle` 新增 `child_turn_cancels`，
  与 `SessionState` 共享。
- **`crates/web/src/api.rs`**：`SubagentSteerBody` + `post_subagent_steer` handler
  （404/409 守卫，admit steer，fire turn-cancel）。
- **`crates/web/src/lib.rs`**：注册路由 `POST /api/sessions/:id/subagents/:task_id/steer`。

### Client

- **`crates/client/src/remote.rs`**：`steer_subagent(session_id, task_id, prompt, images)`。

### TUI

- **`crates/tui/src/key_handler.rs`**：`SubagentSteer(String)` variant，
  `subagent_focused: bool` 参数；聚焦 subagent 时 Enter 路由到 SubagentSteer。
- **`crates/tui/src/subagent_input.rs`**（新文件）：`admit_subagent_steer()` +
  `fire_subagent_turn_cancel()` 辅助函数。
- **`crates/tui/src/app.rs`**：模块注册、`child_turn_cancels` 克隆、subagent 聚焦时
  `display_steers`/`display_queue`/`input_disabled` 计算、`SubagentSteer` 分支、
  `SteerSubmit` 处理器 fire turn-cancel。
- **`crates/tui/src/render.rs`**：disabled 提示文本改为 "subagent ended — esc to return"。

## 测试

### Web API（`crates/web/tests/subagent_steer_api.rs`，新增 5 例）

- `steer_running_subagent_returns_ok` — Running → 200 + admitted_seq > 0 + steer 入 child queue
- `steer_completed_subagent_returns_409` — Completed → 409
- `steer_failed_subagent_returns_409` — Failed → 409
- `steer_cancelled_subagent_returns_409` — Cancelled → 409
- `steer_nonexistent_task_returns_404` — 未知 task_id → 404

### Session（`crates/session/tests/subagent_steer.rs`，新增 3 例）

- `turn_cancel_helpers_work` — SharedCancel token 的 fresh→cancelled→reset 行为
- `turn_cancel_allows_loop_to_continue` — turn_cancel 预先触发 + admit steer →
  SteerConsumed + Done（loop 在 interrupt 后继续）
- `turn_cancel_not_set_behaves_normally` — 无 turn_cancel 的 session 行为不变

### TUI（新增 9 例）

`crates/tui/src/subagent_input.rs`（7 例）：
- `admit_steer_to_running_subagent` — 成功 admit + steer_items 更新
- `admit_steer_to_done_subagent_returns_false` — 已结束 → 拒绝
- `admit_steer_with_no_focus_returns_false` — 无聚焦 → no-op
- `admit_steer_with_empty_text_returns_false` — 空文本 → 拒绝
- `fire_turn_cancel_fires_for_running_subagent` — token 被取消
- `fire_turn_cancel_noop_when_token_not_registered` — 无 token → no-op
- `fire_turn_cancel_noop_for_done_subagent` — 无 blocks → no-op

`crates/tui/src/key_handler.rs`（2 例）：
- `enter_produces_subagent_steer_when_focused` — 聚焦时 → SubagentSteer
- `enter_produces_steer_when_running_and_not_subagent_focused` — 非聚焦 → Steer

### 回归

- `cargo test --workspace`：全量通过，0 failures
- `cargo clippy --workspace`：无 warning

## 风险与回退

- turn_cancel 使用 `std::sync::Mutex`（非 `tokio::sync::Mutex`），仅在非 async 上下文
  短暂持有（lock → cancel/drop），不跨 `.await`，无死锁风险。
- child_turn_cancels 在 subagent 结束时主动移除，无泄漏。
- 若需回退，删除 `turn_cancel` 字段及 `run_loop` 中两处检查即可恢复原有行为。
