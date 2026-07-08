Commit: (working-tree, pre-initial-commit)

# follow-up 队列消费镜像根修 + 删除/重排完整性

## 背景
队列面板上线后 review 发现三类问题：
1. runner 在 idle 边界消费 queue 项后，TUI 的 `queue_items` 镜像不收缩——被消费的行一直以 `[queued]` 显示直到 `Done`，用户以为队列堆积。
2. TUI 点 ✕/▲▼ 时先改本地 `queue_items` 再调 store；store 失败时本地与库不一致（M2）。
3. `delete_input` 无 `promoted_seq` 守卫，理论上可删掉已提升行（M3）。

## 变更

### Part A — 删除/重排完整性（修 M2/M3）
- **M2 本地变更门控**（`crates/tui/src/app.rs`）：✕ 删除与 ▲▼ 重排的本地 `queue_items` 变更改为**仅在 store 操作 `is_ok()` 时**执行；store 失败则本地不动，镜像与库一致。
- **M3 已提升守卫**（`crates/store/src/libsql_store/inputs.rs::delete_input`）：DELETE 加 `AND promoted_seq IS NULL`，已提升（已 drain 为 steer）的行不被删，保留审计轨迹。
- 新测试 `delete_input_preserves_already_promoted_audit_row`（`store/tests/inputs_integration.rs`）。

### Part B — 消费镜像根修（修 M1，从源头而非 TUI 补丁）
- **claim 返回 seq + 原子化**（`crates/store`）：`claim_next_queue` 返回类型 `Option<SessionInput>` → `Option<(i64, SessionInput)>`，把被消费行的 row seq 带回 runner；事务改 `BEGIN IMMEDIATE`（`TransactionBehavior::Immediate`），SELECT+UPDATE 持写锁，并发 claimer 串行排队而非撞 `SQLITE_BUSY_SNAPSHOT`，使「原子消费」名副其实。trait 与 libsql 实现同步更新。
- **新事件**（`crates/session/src/runner.rs`）：`SessionEvent::QueueConsumed { seq: i64 }`——idle 边界消费 queue 项时 emit，携带被消费行的 seq。
- **TUI 收缩镜像**（`crates/tui/src/app.rs`）：收到 `QueueConsumed` 即 `queue_items.retain(|(s,_)| s != seq)`，被消费行即时消失，不再等到 `Done`。
- **其它前端 no-op**：cli `run.rs`、tui `chat.rs`（回放路径）对 `QueueConsumed` 显式 no-op；web `handle.rs` 映射为 `queue_consumed` SSE 步骤事件（供未来 web 队列视图）。

> 选择「根修 + 事件」而非「TUI 在 `Done` 后整表刷新」：前者在每条消费点即时收敛，多 queue 连续消费时每条即时消失；后者要等到整轮结束才一次性收缩，期间仍显示陈旧行。

## 涉及文件
- `crates/session/src/runner.rs` — `SessionEvent::QueueConsumed`（第 38 行）+ emit 点（第 179 行）
- `crates/store/src/store.rs` — trait `claim_next_queue` 返回类型
- `crates/store/src/libsql_store/inputs.rs` — `claim_next_queue` 实现返回 seq + `delete_input` 加 promoted_seq 守卫
- `crates/store/src/libsql_store/mod.rs` — Store impl 接线
- `crates/tui/src/app.rs` — `QueueConsumed` 收缩镜像 + 删除/重排本地变更门控 store `is_ok()`
- `crates/tui/src/chat.rs` — 回放路径 `QueueConsumed` no-op
- `crates/cli/src/run.rs` — `QueueConsumed` no-op
- `crates/web/src/handle.rs` — `queue_consumed` SSE 映射

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 删除不波及已提升行 | `delete_input_preserves_already_promoted_audit_row` | `store/tests/inputs_integration.rs` |
| claim 返回 seq + 标记 promoted + 再删幂等 | `claim_next_queue_returns_seq_marks_promoted_and_idempotent_delete` | `store/tests/inputs_integration.rs` |

## gate
- `cargo test --workspace` → 258 passed / 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## 相关文档
- [agents/session](../../../agents/session/index.md) — run_loop idle 消费 + QueueConsumed
- [agents/store](../../../agents/store/index.md) — claim_next_queue 返回类型
