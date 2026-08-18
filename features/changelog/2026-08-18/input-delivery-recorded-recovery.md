Commit: (working-tree, post-860831d)

# 输入投递 recorded 状态机：F1 claim 拆分 + F2 逐条落账 + F3 有界 drain 重启 + F4 提交失败可见

## 背景

web `POST /prompt` admit 成功即对客户端承诺消费该输入，但投递链路存在四个丢失/静默窗口：
1. **F2（promote→consume 缝隙）**：steer/queue 行被 promote 后若 run 崩溃或硬取消（POST /stop），行停留在 promoted 态——下一次 drain 只轮询 pending，输入被永久搁浅且无人知晓。
2. **F3（drain 失败即弃）**：drain run 以 Err 结束时（LLM 5xx、store 抖动），尚待消费的 steer/queue 行被直接抛下，admit 的承诺落空。
3. **F1（claim 竞速窗口）**：`claim_steers` 原把只读 `pending_inputs` 与变更 `promote_inputs` 包进同一个 `tokio::select!`——biased select 在硬取消时可能事务中途丢弃 promote future，造成「已提交提升但调用方不知情」的搁浅行。
4. **F4（TUI 静默失败）**：TUI Enter steer 在 store 层 admit 失败时无任何反馈，输入文字随 composer 清空而消失。

## 变更

### session：F1 claim 拆分（消除竞速搁浅源）
- **`crates/session/src/runner/steer.rs`**：`claim_steers` 的 select 拆为两段——只读 `pending_inputs` 轮询保留 cancel-guard（db_lock 争用不得阻塞硬取消，放弃读无害：行保持 pending）；变更 `promote_inputs` 移出 select、不与硬取消竞速（其 BEGIN→UPDATE→COMMIT 手工事务被 biased select 中途丢弃会造搁浅行）。语义对齐 `claim_one_queued`。

### store：recorded 标记列 + v10 迁移
- **`crates/store/src/libsql_store/schema.rs`**：`session_inputs` 新增 `recorded INTEGER NOT NULL DEFAULT 0`（SCHEMA_VERSION 9→10）；迁移回填——列落地时已 promoted 的历史行视为已消费（audit 语义，已在 transcript 中）。
- **`crates/store/src/store.rs`**：`Store` trait 新增 `mark_inputs_recorded`（幂等落账）与 `recover_orphan_inputs`（promoted 且未 recorded 的孤儿行翻回 pending，返回行数），默认 no-op 保测试 fake 编译。
- **`crates/store/src/libsql_store/inputs.rs`**：libsql 实现；re-promote 时重置 recorded 标记。

### session：F2 入口恢复 + 逐条落账
- **`crates/session/src/runner/input_recovery.rs`**（新，57 行）：`recover_orphaned_inputs`（run 入口回收孤儿行）、`mark_input_recorded`（消费一条落账一条）、`unpromote_batch`（失败批次整体翻回 pending）。
- **`crates/session/src/runner/mod.rs`**：`entry_drain_mode` 轮询前先 `recover_orphaned_inputs`（单写者不变量保证安全）；run Err 后 best-effort `reabsorb_tail`，不遮蔽原始错误。
- **`crates/session/src/runner/drain.rs`**：queue 消费循环改为**逐条** mark（对齐 steer 循环）——批次中途失败时，已消费项已落账、失败项与剩余项被 unpromote，重跑只补缺口。

### web：F3 有界 drain 重启
- **`crates/web/src/handle.rs`**：drain 循环加 `MAX_DRAIN_RESTARTS=2` 有界重启——仅当 run Err、仍有 pending 输入、未被硬取消（POST /stop 语义优先，被取消的 drain 永不复活）且预算未尽时重试，250ms 退避；sink/flusher/tx 跨重试存活保证事件仍持久化与广播。`pending_input_count` 把 store 读错误视为零（不可读 store 不得被误判为"仍欠输入"而复活失败 drain）。内嵌 tests 拆出。
- **`crates/web/src/handle_tests.rs`**（新，145 行）：原 handle.rs 内嵌 5 个单测经 `#[path]` 拆出，保运行时文件低于行数上限（纯移动）。

### tui：F4 提交失败可见
- **`crates/tui/src/steer_fire.rs`**：`flash_on_admit_failure(Option<i64>)` 把 admit 结果映射为失败 flash（成功 None）；`STEER_SUBMIT_FAILED_FLASH = "⚠ steer submit failed — recover text with ↑ history"`。
- **`crates/tui/src/app.rs`**：`KeyAction::Steer` 分支接线——store 层失败即设 `mode_flash`；`push_history` 在每次提交（含失败）都会执行，原文必经 ↑ 历史可恢复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| recorded 状态机 pending→promote→mark | `recorded_state_machine_pending_promote_mark` | `crates/store/tests/inputs_recorded.rs` |
| re-promote 重置标记 | `promote_resets_recorded_marker_on_repromotion` | 同上 |
| 仅回收未落账的 promoted 行 | `recover_orphan_inputs_recovers_only_unrecorded_promoted_rows` | 同上 |
| v9→v10 迁移回填 | `migration_v9_to_v10_backfills_recorded_for_promoted_rows` | 同上 |
| 孤儿行入口重吸收 | `orphan_recovery_reabsorbs_promoted_unrecorded_input` | `crates/session/tests/input_delivery_recovery.rs` |
| run Err 后重吸收 pending queue | `run_err_still_reabsorbs_pending_queue` | 同上 |
| 硬取消不丢 promote 也不复活 | `steer_claim_survives_hard_cancel_without_lost_promote` | 同上 |
| 失败且有 pending 时有界重启 | `drain_error_with_pending_inputs_restarts_bounded` | `crates/web/tests/drain_restart_on_error.rs` |
| 重启捞回搁浅输入 | `drain_restart_recovers_stranded_pending_inputs` | 同上 |
| handle 订阅者逐出/release 语义（拆分回归） | `release_subscriber_evicts_creator_handle_when_last_and_idle` 等 5 项 | `crates/web/src/handle_tests.rs` |
| steer 批次恢复适配 | 既有测试适配 | `crates/session/tests/steer_batch_recovery.rs` |
| F1 预触硬取消下 steer 保持 pending 不丢 | `pre_fired_hard_cancel_leaves_steer_pending_not_lost` | `crates/session/src/runner/steer.rs`（内嵌） |
| F4 admit 结果→flash 映射 | `flash_on_admit_failure_none_on_success` / `flash_on_admit_failure_some_on_store_failure` | `crates/tui/src/steer_fire.rs`（内嵌） |

- 全量回归（当次实跑，2026-08-18 23:52，含同工作树全部功能）：`cargo test --workspace` **2961 passed / 0 failed / 0 ignored**（187 suites）、clippy `-D warnings` 零警告、`cargo build --workspace` 干净。
- 行数：新文件 57–338 行 ≤400；`handle.rs` 拆分后 704 行 ≤800。

## Impact Surface
- web 客户端：admit 成功的输入在瞬时故障下最终必被消费（或有界放弃后可查 pending），不再静默丢失。
- `Store` trait 新增两个带默认实现的方法——自定义实现无需改动即可编译。
- 不影响：CLI 单 prompt 路径、消息/事件 schema。TUI 侧 Enter steer 提交失败新增 flash 反馈（F4），交互语义不变。

## Related Docs
- [agents/store](../../agents/store/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/session](../../agents/session/index.md)
