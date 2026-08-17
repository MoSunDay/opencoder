Commit: (working-tree, post-1ba8f426)

# queue-drain 语义修复：一次弹一条 + 触发去自续 + 模块拆分

## Context

drain 模式（空 prompt 续跑：web 两段式 delivery / 纯 skill 提交）存在三类缺陷：① 旧 `drain_queued` 单次调用内循环弹多条 queue 项，bare 控制命令经 `continue` 连续弹出，外层 run_loop 在两次弹出之间**看不到中断/新 steer**；② 纯 skill 触发注入条件挂在 session 的粘性 skill 上——queue/steer 重启带着陈旧 active skill 时会为**从未提及该 skill** 的条目再注入 `SKILL_TRIGGER`，放大 drain 自续循环；③ `claim_next_queue` 与硬取消的 biased select 竞态可能在 COMMIT 中途丢弃 future，造成条目「已提升未记录」的永久丢失。另 `steer.rs`（818 行）/`runner/mod.rs`（815 行）超 800 行迭代上限。

## Change Summary

- **`crates/session/src/runner/steer.rs`（818→273 行）拆出 `runner/drain.rs`（332 行，新文件）**：queue 侧消费 + drain 步进归 drain.rs，steer 侧 claim/peek 留 steer.rs——按 Queue/Steer 功能边界拆分，各自 ≤800。
  - `claim_one_queued`：claim 事务（BEGIN IMMEDIATE → SELECT → UPDATE → COMMIT，<1ms）**不再挂 cancel-guard select**（竞态丢 COMMIT = 永久丢数据）；瞬时 Err 恰好重试一次（warn 不静默），持续失败落 None。
  - `drain_one_queued`：**每次至多弹一条**，bare 控制命令内联应用后返回 `ControlCmd`（调用方跳过 LLM、下一迭代再弹下一条），控制命令应用失败时 `unpromote_inputs` 回退该条目待下轮重试。
  - `idle_drain`：弹空后补查「SELECT 与 peek 间隙新入队」的条目真消费（裸 Continue 会让 run_loop 顶部只查 steer 不查 queue，思考期搁浅条目）；`entry_drain_mode` 承接 drain 入口判定 + 触发注入 + pending 优先。
  - `MAX_CONSUME_STREAK=32`：连续 ConsumeNext 热自旋上限，超限强制 Done，前端 resync 慢速重启自愈；`reabsorb_tail` 有界收尾。
- **`crates/session/src/runner/mod.rs`（815→786 行）**：run_loop 改用 `drain::*`；测试夹具拆出 `runner/test_fixtures.rs`（`#[cfg(test)]`）；集成侧 `FlakyClaimStore` 移入 `tests/common/mod.rs`（skill_queue_drain.rs 444→309 行，回 400 上限内）。
- **`crates/session/src/skill_resolve.rs`**：`record_compound` 纯 skill 触发注入改为**本次输入真的解析过 ≥1 个 `$skill` token**（`resolved_now`，含 discovery 匹配 + 未解析排除）才注入——粘性 skill 不再为未提及它的 queue/steer 条目注入触发，斩断自续循环。
- **`crates/session/src/runner/subagent.rs`**：子 agent 强制 `autopilot.mode = Off`——子 agent 自跑 PLAN→ACT→VERIFY 或 review pass 会与父循环双重驱动、回填失步。
- **`crates/session/tests/skill_queue_drain.rs`（新，309 行）**：4 个集成测试（MockChatClient + open_memory + tempdir，无真 LLM/网络）。

## Validation（当次实跑）

- `cargo test --workspace --no-fail-fast`：175 个套件全绿，合计 **2855 passed / 0 failed**（复跑取证口径；首跑 2852，其间并行迭代的 task-plan 测试 +3。上一基线 2804 不降反升；含本轮新增 4 条 drain 集成测试）。`-p opencoder-session` 口径 73 套件 643 passed / 0 failed；`steer_reabsorb::late_steer_reabsorbed_after_run_loop_returns` 通过（入口预查短路化后 poll 预算复原，P1-4 契约回归绿）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（exit 0）。
- `cargo build --workspace`：编译干净（Finished dev profile）。
- 行数合规：steer.rs 273 / drain.rs 332 / mod.rs 786 / test_fixtures.rs 新增 / skill_queue_drain.rs 309，全部 ≤ 上限。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `skill_queue_drain.rs::sticky_skill_with_pending_queue_pops_queue_first` | integration | 粘性 skill 挂起时 pending queue 优先弹出（pending-first），skill 不吞 queue |
| `skill_queue_drain.rs::cancel_then_restart_pops_queue_exactly_once` | integration | 取消后重启恰弹一次（不重复消费、不丢条目） |
| `skill_queue_drain.rs::sticky_skill_empty_prompt_no_pending_still_triggers` | integration | 空 prompt 无 pending 时粘性 skill 仍触发（触发注入未被误杀） |
| `skill_queue_drain.rs::transient_claim_err_retries_once_and_pops` | integration | 瞬时 claim Err 恰好重试一次后正常弹出（不搁浅） |

无删测试 / 无 `#[ignore]` / 无弱断言 / 无密钥。
