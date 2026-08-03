Commit: (working-tree, pre-initial-commit)

# feat(session): /clear 有 plan 时保留 plan，无 plan 时回退空白重置

## 背景

`/clear`（`ControlCmd::ClearContext`，别名 `/act_clear_context`）此前无条件将
会话折叠为一条空白 fresh-start marker。这意味着即使用户刚在 plan 模式下制定好
计划，执行 `/clear` 会把计划也一并丢弃，act agent 拿不到任何可执行的指令。

本次改为：当 transcript 中存在 plan（最近一条非空 assistant 消息）时，`/clear`
走 plan->act handoff 路径——将历史折叠为一条携带 plan 的合成指令，使 act agent
可以继续执行该计划；仅当没有任何 plan 可保留时，才回退到原先的空白 fresh-start
行为。

## 变更

### `crates/session/src/control_cmd.rs` — ClearContext arm 重写

- **plan-preserve 路径**：调用 `crate::plan_handoff::handoff(session, "")`，
  若返回 `Some(display)` 则折叠为携带 plan 的合成指令并发出
  `SessionEvent::PlanHandoff(display)`；`handoff_plan` 字段保存真实 plan 文本。
- **fallback 路径**（无 plan）：收集 head images、构建 `fresh_start_message()`，
  调用 `session.after_handoff(store_msg_count, CLEAR_CONTEXT_SENTINEL)`，使 resume
  时重建空白 marker 而非 plan 指令。
- **新增常量与辅助**：
  - `CLEAR_CONTEXT_SENTINEL`（`"<<OPENCODER_CLEAR_CONTEXT_MARKER>>"`）——fallback
    路径存入 `handoff_plan`，resume 据此重建空白 fresh-start。
  - `is_clear_context_handoff(handoff_plan) -> bool`——判断是否为 sentinel。
  - `CLEAR_CONTEXT_BODY`（`"[Context cleared - starting fresh in act mode.]"`）。
  - `fresh_start_message()`——构建合成 `Message::user`（`synthetic = true`）。
  - `persist_clear(session)`——持久化 `agent = "act"` + handoff_seq + handoff_plan。
- 两条路径之后统一：切换到 act agent、清除 skill、`persist_clear`、发出
  `AgentSwitch("act")` 与 `TranscriptReset`。
- **附带**：`SwitchAgent("plan")` 现在重置 `plan_input_count = 0`，与新 plan
  会话的计数语义对齐。

### `crates/session/src/resume.rs`

未改动——`is_clear_context_handoff` 谓词已覆盖两条路径（sentinel 走空白重建，
真实 plan 走 handoff 重建），resume 逻辑无需变更。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ClearContext 有 plan -> 折叠为 handoff 指令、保留 plan 文本（apply 直调） | `apply_clear_context_collapses_and_emits` | `crates/session/src/control_cmd.rs` (unit) |
| ClearContext 无 plan -> 回退空白 fresh-start、存 sentinel（apply 直调，新增） | `apply_clear_context_no_plan_falls_back_to_fresh_start` | `crates/session/src/control_cmd.rs` (unit) |
| sentinel 谓词：sentinel 为 true、plan 文本/空串为 false | `clear_context_sentinel_predicate` | `crates/session/src/control_cmd.rs` (unit) |
| `/act_clear_context` 解析为 ClearContext | `parse_exact_matches` | `crates/session/src/control_cmd.rs` (unit) |
| 有 plan 时 ClearContext -> resume 后指令携带 plan 文本 | `clear_context_survives_resume` | `crates/session/tests/control_cmd.rs` |
| 无 plan 时 ClearContext -> resume 后为空白 fresh-start marker（新增） | `clear_context_no_plan_survives_resume` | `crates/session/tests/control_cmd.rs` |
| sentinel 不泄露到 model context；fresh-start body 在上下文中 | `clear_context_sentinel_never_reaches_model_context` | `crates/session/tests/control_cmd.rs` |

- `cargo test -p opencoder-session --lib control_cmd` -> **8 passed** / 0 failed（含 1 新增）
- `cargo test -p opencoder-session --test control_cmd` -> **6 passed** / 0 failed（含 1 新增）
- 全量回归 `cargo test --workspace` -> **1669 passed / 0 failed / 1 ignored**（预先存在）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` -> 零警告

## Impact Surface

- **行为变更**：`/clear` 不再无条件清空——有 plan 时保留为 handoff 指令。
  仅无 plan 时保持原有空白 fresh-start 语义。
- 接缝不变：`Store` / `ChatStream` 抽象、resume 重建逻辑、runner apply 入口均未改动。
- `CLEAR_CONTEXT_SENTINEL` 仅用于持久化层标记，不会进入发给 LLM 的 context。

## Related Docs

- [agents/session](../../agents/session/index.md) — control_cmd apply、resume 重建
