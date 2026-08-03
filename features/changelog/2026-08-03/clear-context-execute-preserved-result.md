Commit: (working-tree, pre-initial-commit)

# fix(session): /act_clear_context 保留结果后立即执行，不再短路停止

## 背景

`/act_clear_context`（`ControlCmd::ClearContext`）此前在 `run_with_registry` 中
被 parse 后直接 `return Ok(())` 短路返回，从不进入 `run_loop`。虽然
`control_cmd::apply(ClearContext)` 内部已通过 `plan_handoff::handoff` 将最后的
assistant 消息保留为合成 user 指令，但因短路返回该指令从未被执行——用户看到的是
"清空了全部上下文，但什么都没做"。

本次修复：ClearContext 在保留了结果（`handoff_plan` 不是 sentinel）时，不再短路
返回，而是 fall through 到 `run_loop`，让 agent 拿着保留的 handoff 消息立即开始
执行。仅当没有可保留的结果时（sentinel 路径）才保持原有的短路停止行为。

## 变更

### `crates/session/src/runner/mod.rs` — ClearContext 执行路径修复

- **idle 入口路径**（`run_with_registry`，~L76-90）：parse 到 ClearContext 后，检查
  `handoff_plan` 是否为 sentinel。非 sentinel（有保留结果）→ 清空 `user_text`（避免
  原始命令字符串被 record 为 user 消息）、fall through 到 `run_loop` 执行。Sentinel
  路径（无结果）→ 保持原有 `return Ok(())` 短路行为。
- **queue-draining 路径**（`run_loop` 内部，~L290-303）：队列中的 ClearContext 在保留
  结果时设置 `got_real_prompt = true` 并 break 到外层循环触发 LLM 执行，而非
  `continue` 继续排空队列。
- **注释更新**：控制命令注释区分 `/act`、`/plan`（仍然短路）与
  `/act_clear_context`（保留结果时 fall through 执行）。

### `crates/session/tests/control_cmd.rs` — 测试适配与新增

- **更新 `clear_context_survives_resume`**：mock 推入 `done_turn("done")` 响应执行
  turn；`session.messages.len()` 从 1 改为 2（handoff + assistant 响应）；resume
  `messages.len()` 同步改为 2。
- **新增 `clear_context_executes_preserved_result`**：验证有 assistant 结果时
  `/act_clear_context` 触发恰好一次 LLM 调用、保留的结果文本出现在 model context 中、
  原始命令字符串不泄露给 model。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 有结果时 ClearContext 触发一次 LLM 调用、结果在 context 中、命令不泄露（新增） | `clear_context_executes_preserved_result` | `crates/session/tests/control_cmd.rs` |
| 有结果时 ClearContext + 执行 → resume 正确重建 handoff + 响应（更新） | `clear_context_survives_resume` | `crates/session/tests/control_cmd.rs` |
| 无结果时 ClearContext → sentinel fresh-start，不执行（不变） | `clear_context_no_plan_survives_resume` | `crates/session/tests/control_cmd.rs` |
| sentinel 不泄露到 model context（不变） | `clear_context_sentinel_never_reaches_model_context` | `crates/session/tests/control_cmd.rs` |

- `cargo test -p opencoder-session --test control_cmd` → **7 passed** / 0 failed
- 全量回归 `cargo test --workspace` → **全绿**
- clippy `cargo clippy --workspace --all-targets -- -D warnings` → **零警告**

## Impact Surface

- **行为变更**：`/act_clear_context` 在有可保留结果时不再只清空——会保留最后结果并
  立即执行。用户感知：命令不再"静默清空后什么也不做"。
- 无结果时（无 assistant 消息）行为不变：仍为空白 fresh-start + 短路停止。
- 接缝不变：`Store` / `ChatStream` / resume 重建 / control_cmd::apply 均未改动。

## Related Docs

- [agents/session](../../agents/session/index.md) — runner run_with_registry、run_loop
- [既有 changelog: clear-context-preserve-plan](clear-context-preserve-plan.md) — 首次引入 plan 保留逻辑
