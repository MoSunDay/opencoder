Commit: (working-tree, pre-initial-commit)

# 修复队列消息在 thinking 阶段丢失的 4 个缺陷

## 背景

用户报告队列消息（queued messages）在 thinking 阶段有概率丢失。根因分析追踪到 4 个独立缺陷，均位于 session drain 逻辑与 web reaper 路径，导致队列准入（queue admission）未被消耗即被跳过。

## 变更

### idle_drain 消耗 late queue（Fix 1 — 主要）
- **`crates/session/src/runner/steer.rs`** `idle_drain` Empty 分支：原代码仅 `has_pending_queues` peek 后返回 `Continue` 但不消耗该项。调用者回到 `run_loop` 顶部时 `claim_steers` 不检查队列、`drain_mode=false` 跳过 drain → 触发虚假 LLM 调用 → 队列项在 thinking 期间搁置。修复：检测到 late queue 时实际调用 `drain_one_queued` 消耗它（Prompt→Continue, ControlCmd→SkipLlm, Empty→fallthrough）。

### claim_one_queued 移除取消防护（Fix 2 — 防数据丢失）
- **`crates/session/src/runner/steer.rs`** `claim_one_queued`：移除 `biased tokio::select! { cancel_guard(hard), claim_next_queue }` 包装。原代码在 hard cancel 触发时可能 drop future 于 COMMIT 执行期间，导致项被永久提升（不可见）但未记录为用户消息。事务在本地 SQLite <1ms 完成；run_loop 顶部中断检查在下一次迭代处理 cancel。

### web reaper 超时延长（Fix 3）
- **`crates/web/src/handle.rs`** reaper 轮询循环：`for _ in 0..100`（5s）→ `for _ in 0..12_000`（10 min）。真实 thinking 阶段持续 10-60s+，5s 上限使深度防御重启永不触发。

### drain_mode_step late-check 前置（Fix 4）
- **`crates/session/src/runner/steer.rs`** `drain_mode_step` Empty 分支：将 `has_pending_steers || has_pending_queues` 检查移至 `needs_llm` 提前返回之前，确保 drain 模式下也能消耗 late queue 项。

### 回归测试
- `idle_drain_consumes_pending_queue`：队列项被消耗、记录为用户消息、不再 pending。
- `idle_drain_empty_queue_no_gate_returns_done`：空队列无 gate → Done。
- `claim_one_queued_completes_under_hard_cancel`：pre-fire hard cancel 后 claim 仍完成（旧代码此测试 FAIL）。
- `drain_mode_step_proceeds_when_transcript_ends_with_tool_result`：drain 模式下转写以 Role::Tool 结尾时返回 Proceed（非 Idle），确保模型处理工具结果。
- `drain_mode_step_idles_when_transcript_ends_with_assistant`：drain 模式下转写以 Assistant 结尾时返回 Idle。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| idle_drain 消耗队列项 | idle_drain_consumes_pending_queue | steer.rs |
| idle_drain 空队列 Done | idle_drain_empty_queue_no_gate_returns_done | steer.rs |
| claim 完成 under hard cancel | claim_one_queued_completes_under_hard_cancel | steer.rs |
| drain_mode 处理 Tool 结果 | drain_mode_step_proceeds_when_transcript_ends_with_tool_result | steer.rs |
| drain_mode Assistant 空闲 | drain_mode_step_idles_when_transcript_ends_with_assistant | steer.rs |

- steer 全量：`cargo test -p opencoder-session --lib steer::tests` → 18 passed
- runner 模块：`cargo test -p opencoder-session --lib runner::` → 48 passed
- web：`cargo test -p opencoder-web` → 81 passed（单元 11 + 多集成套件）
- clippy：`cargo clippy -p opencoder-session -p opencoder-web --all-targets -- -D warnings` → 零警告
- 行数：steer.rs 785 ≤ 800；handle.rs 614 ≤ 800

## Impact Surface
- 修复 thinking 阶段队列消息丢失（用户可感知：排队消息不再消失）
- 不影响：CLI/Store/API 签名；变更仅限于 session drain 逻辑内部与 web reaper 轮询上限

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/web](../../agents/web/index.md)
