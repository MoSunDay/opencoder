Commit: (working-tree, pre-initial-commit)

# fix(session,store): steer batch 错误传播 + 失败恢复（P1-2 / P1-3，附带 P2-6）

## 背景

此前 `control_cmd.rs` 中的 `persist_agent` 与 `persist_clear` 用 `let _ =`
静默吞掉了 store 错误。这意味着：模式切换（如 `/plan` steering）过程中若
`update_session` 写入失败，错误会被无声丢弃——内存中的 agent 已切换，但持久化
记录并未落库，造成「内存态与持久态分叉」。

更严重的是**没有任何恢复路径**：批量 steer 的 drain 循环里，已 promote 的 item
一旦在 `apply()` 时失败就**永久滞留**——它已被标记为 promoted，下一轮 drain
不会再选中它，于是这条用户输入再也不会被处理。

本次把这两个问题一并修复：让错误可见（传播），并让失败项可重试（unpromote 回
pending）。

## 变更

### 错误传播（P1-2）—— `crates/session/src/control_cmd.rs`

- `persist_agent`（~169）与 `persist_clear`（~186）：`let _ = store.update_session(...)`
  改为 `store.update_session(...).await?`，错误向上传播；返回类型由 `()` 改为
  `Result<()>`。
- `apply()`（~98）：`persist_agent(session, name).await?`——`?` 将 store 错误
  一直传播到 runner 的 drain 循环，使其能据此进入恢复分支。

### 批量恢复（P1-3）—— store 抽象 + 实现

- **`crates/store/src/store.rs`**（~79）：新增 trait 方法
  `unpromote_inputs(&self, session_id: &str, seqs: &[i64]) -> Result<()>`，
  提供**默认 no-op 实现**（直接返回 `Ok(())`）。这样 test fake / 任何不实现该
  方法的 Store 都向后兼容，无需 override。语义：把已 promote 的 input 重置回
  pending（待消费）状态。
- **`crates/store/src/libsql_store/inputs.rs`**（~90）：新增 `unpromote()` 函数——
  幂等 `UPDATE`，仅对当前已 promote（`promoted_seq IS NOT NULL`）的行置
  `promoted_seq = NULL`，未 promote 的行不动。
- **`crates/store/src/libsql_store/mod.rs`**（~186）：`LibsqlStore` 实现
  `unpromote_inputs`，委托给 `inputs::unpromote()`。

### 批量恢复（P1-3）—— runner drain 循环

- **`crates/session/src/runner/mod.rs`**（~219）：批量 steer drain 循环中，当
  `apply()` 返回 `Err` 时，收集**失败项 seq 及其后所有未处理 seq**
  （`steer_prompts[idx..]`），调用 `unpromote_inputs` 将它们一并重置为 pending，
  再向上传播该错误。如此下次 run 会从失败点开始重试整批。
- **`crates/session/src/runner/steer.rs`**（~189）：`drain_one_queued` 中，当
  `apply()` 返回 `Err` 时，unpromote 当次单独 claim 的那条 item，使下次 run
  能重试它。

### 清理

- **`crates/store/src/libsql_store/sessions.rs`**：删除无用的 dead code
  `encode_cursor` 函数。

### 附带修复（P2-6）—— autopilot iteration 饱和

- **`crates/session/src/runner/event.rs`**（~288）：`from_sse` 对 `"autopilot"`
  事件的 `iteration` 解析，由 `data.get("iteration")?.as_u64()? as u32`
  （u64→u32 静默截断环绕）改为 `.min(u32::MAX as u64) as u32`（饱和转换），
  避免 `u64::MAX` 被环绕成一个极小的错误值。

### 附带修复 —— worker 持久化错误日志

- **`crates/tui/src/worker.rs`**（~278、~290）：`persist_session_agent`（即
  `persist_agent`）现在出错时记录 `tracing::warn!`，而非静默丢弃：
  `if let Err(e) = persist_session_agent(sess, &name).await { tracing::warn!(...) }`。
  注意：worker 处于 UI 循环、不能像 runner 那样向上传播错误，故以 warn 落日志
  保证可观测，而非静默。

## 测试清单（crates/session，integration + unit）

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Store unpromote 重置已 promote 的 input | `steer_batch_failure_unpromotes_remaining_items` | `session/tests/steer_batch_recovery.rs` |
| 部分 batch 失败只 unpromote 剩余（已成功的不动） | `partial_batch_failure_unpromotes_only_remaining` | `session/tests/steer_batch_recovery.rs` |
| Runner batch 失败：persist_agent 错误传播 + 全量 unpromote | `runner_consumes_batch_steers_with_failing_store` | `session/tests/steer_batch_recovery.rs` |
| 迟到 steer 在 run_loop 返回后被重新吸收 | `late_steer_reabsorbed_after_run_loop_returns` | `session/tests/steer_reabsorb.rs` |
| autopilot iteration u64::MAX 饱和为 u32::MAX | `from_sse_autopilot_large_iteration_saturates` | `session/src/runner/event.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **全绿**（session lib `RUST_TEST_THREADS=1`
  268 passed，EXIT=0）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- build：`cargo build --workspace` → 编译干净（EXIT=0）。
- 行数：新增/修改文件均未超限（control_cmd.rs、runner/mod.rs、steer.rs、
  event.rs、store.rs、libsql_store 各文件均在 800 行内）。

## 影响面

- **用户**：`/plan` 等 steer 在 store 写入失败时不再静默丢失——错误向上传播
  可见，失败批量在下次 run 时从失败点整批重试，已 promote 项不会永久滞留。
- **不影响**：CLI 入口、`Store` trait 默认实现（向后兼容——test fake 与未实现
  `unpromote_inputs` 的 Store 无需 override，默认 no-op）。
- worker 路径：UI 循环不能传播错误，故以 `tracing::warn!` 替代静默，保证可观测。

## Related Docs

- [session 模块](../../../agents/session/index.md)
- [store 模块](../../../agents/store/index.md)
