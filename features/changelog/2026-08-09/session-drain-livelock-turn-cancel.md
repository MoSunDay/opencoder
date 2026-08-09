Commit: (working-tree, pre-initial-commit)

# fix(session): claim_* 不再观测 turn_cancel，消除 drain-loop 活锁

## 背景

`claim_steers` / `claim_one_queued` 各自携带一个 `turn_cancel` cancel-guard 分支：
当 token 已 fired 时，DB 读取被短路并返回空。但 `has_pending_steers` /
`has_pending_queues` **没有**同样的 guard。于是当存在 pending input 且
`turn_cancel` 已 fired（未被重置）时，出现致命不一致：

- `claim_*` 报「空」（被 turn_cancel 短路）
- `has_pending_*` 报「有」

drain 主循环在 `ConsumeNext` 上无限自旋——没有 tool call → doom-loop 守卫
（`DOOM_THRESHOLD=20`）永不触发 → 真正的无界活锁。

`turn_cancel` 的本意只是中止**当前 LLM turn**（pre-fired 时第一次 LLM 调用
按设计中止、之后 `run_loop` 重置它并跑真实 turn）。它不应阻断 input 的
promote/claim。修复后 `claim_*` 只观测 hard（session）cancel，input 正常
promote，循环得以推进。

## 变更

### `crates/session/src/runner/steer.rs`
- `claim_steers`：移除 `turn_cancel` 的 `cancel_guard(turn)` 分支与对应的
  token 快照（`session.turn_cancel` 读取），仅保留 hard cancel 分支。
- `claim_one_queued`：同样移除 `turn_cancel` guard 分支，仅保留 hard cancel。
- `cancel_guard` doc-comment 同步更新：原描述「block cancel/turn_cancel」改为
  「block the session (hard) cancel」。
- 范围：纯删除两条 `select!` 分支 + 两处 token 快照，不改函数签名、不改数据形状。

### `crates/session/tests/drain_mode.rs`
新增两条端到端 drain 回归（复刻活锁场景：pending input + pre-fired turn_cancel +
drain，断言 Done 终止、LLM 至少被调一次）：
- `drain_mode_pending_steer_with_fired_turn_cancel_promotes_it`
- `drain_mode_pending_queue_with_fired_turn_cancel_consumes_it`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| pending Steer + pre-fired turn_cancel 仍被 promote 且 loop 终止（无活锁） | `drain_mode_pending_steer_with_fired_turn_cancel_promotes_it` | `crates/session/tests/drain_mode.rs` |
| pending Queue + pre-fired turn_cancel 仍被 consume 且 loop 终止 | `drain_mode_pending_queue_with_fired_turn_cancel_consumes_it` | `crates/session/tests/drain_mode.rs` |
| claim_steers fired turn_cancel 下仍 promote | `claim_steers_claims_even_when_turn_cancel_fired` | `crates/session/src/runner/steer.rs` |
| claim_one_queued fired turn_cancel 下仍 pop 队列 | `claim_one_queued_claims_even_when_turn_cancel_fired` | `crates/session/src/runner/steer.rs` |
| claim_steers 对 turn_cancel 不可见且幂等 | `claim_steers_ignores_turn_cancel_and_is_idempotent` | `crates/session/src/runner/steer.rs` |

- 全量回归：`cargo test --workspace` → 2216 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 行数：`steer.rs` ≤ 800；`drain_mode.rs` ≤ 800

## Impact Surface
- **修复**：消除「pending input + 已 fired turn_cancel」下的 drain-loop 无限自旋活锁。
- **不变**：turn_cancel 仍按设计中止首个 LLM turn（之后 run_loop 重置）；hard
  session cancel 仍可中断 claim。claim_* / has_pending_* 对外签名与行为契约不变。
- **不影响**：Store / ChatStream / web / CLI 边界。

## Related Docs
- [agents/session](../../agents/session/index.md)
