# todos 排障收尾：M1–M6 + L1/L2/L4/L5 修复、防护测试与 E19b/E19c

## 背景

对 `crates/todos` 完成一轮 bug-sweep 复查后的收尾迭代：修复 6 个 Medium
（M1–M6）与 4 个 Low（L1/L2/L4/L5）缺陷，为每个修复补齐 unit/integration/CLI
测试，并新增 e2e 场景 E19b（DAG 并发 + `--debug` 投影 + events 游标）与
E19c（外部 interrupt → resume 自愈 → rc==1 挂起退出码）。

## 修复清单（修复 → 实现 → 测试）

| # | 缺陷 | 修复位置 | 验证 |
|---|------|----------|------|
| M1 | rewind/中断后迟到 **failed** 结果仍覆盖状态 | `batch.rs::apply_result` Err 分支镜像 Ok 分支守卫：`current != Running` 时记日志丢弃 | `tests/late_results.rs::rewound_sibling_discards_late_failed_result` |
| M2 | `max_attempts` 耗尽后 Blocked 项被永久跳过 | `transitions.rs::candidate(spec)` 接收 spec；attempt 上限 → `Failed` | `tests/transitions_guards.rs::blocked_candidate_at_exhausted_attempts_marks_failed` |
| M3 | dispatch 清空 `last_error`/`next_context_mode`，丢失修订上下文 | `transitions.rs::dispatch` 保留两字段（仅清 candidate） | `tests/transitions_guards.rs::dispatch_keeps_previous_candidate_for_retry_recovery_context` |
| M4 | 父 Session `summary/summary_seq` 被工作流状态 JSON 污染 | `parent.rs::decide` 不再写 summary；`accept(correction)` 参数化纠错反馈 | `tests/boundary_guards.rs::parent_summary_stays_clean_across_decisions` |
| M5 | interrupt 与并发提交撞 generation 冲突即停车 | `runner.rs::interrupt` 有界重试（`INTERRUPT_COMMIT_RETRIES=3`，reload 后重判终态） | `tests/interrupt_retry.rs::interrupt_retries_past_generation_conflict` |
| M6 | 非法父决策/验收提交后才炸；重复里程碑非幂等 | `batch.rs::{validate_acceptance,validate_decision}` 干跑校验 + 验收纠错循环（重试 2 次）；`runner.rs` 全决策先验、重复 `MarkMilestone` 幂等跳过 | `tests/boundary_guards.rs::{nondispatch_decision_correction_reasks,acceptance_correction_reasks_on_failed_gate,duplicate_milestone_remark_is_idempotent}` |
| L1 | `workflow_resumed` 提交未持久化 `Running`，崩溃窗口状态错 | `runner.rs::resume` 该提交写 `status=Running`（不额外 bump generation，保持 CAS +1 不变量） | `tests/boundary_guards.rs::resume_persists_running_before_first_dispatch`；`tests/interrupt.rs::external_interrupt_cancels_inflight_todo_and_is_resumable` 回归 |
| L2 | terminal 挂起时 Running/CandidateReady/Accepting 未归约 | `transitions.rs::terminal(Suspended)` 回滚至 `Interrupted` 并清 candidate | `tests/transitions_guards.rs::suspended_terminal_rolls_back_candidate_ready_and_accepting` + `tests/recovery.rs` 断言更新 |
| L4 | 深依赖链 `reject_cycles` 递归栈溢出 | `domain.rs` 迭代 DFS 显式栈 | `domain.rs` in-file：30k 深链校验通过 |
| L5 | todo id 含 `/`、`..`、`\0` 可逃逸 debug 投影目录 | `domain.rs::validate_spec` 拒绝路径不安全 ID | `domain.rs::validate_spec_rejects_path_unsafe_todo_ids`（in-file，非法 ID 逐一拒绝） |

CLI 补测：`crates/cli/tests/todos_cli_dispatch.rs::events_after_cursor_sees_only_newer_events`
——同一 DB 内 3 事件后，`--after <last>` 为空、`--after <last-1>` 恰余 1 条，
两游标下 CLI dispatch 均 rc=0。

## e2e

- `scripts/e2e/todos_scenarios.py`（E19b/E19c，已在 `scripts/e2e_glm.py` 注册）：
  - **E19b**：3-TODO DAG（依赖阻塞→并发批）+ `--debug` 投影目录断言 + `events --after` 增量游标。
  - **E19c**：运行中外部 `interrupt` → 验证挂起投影 → `resume` 走完 → 校验 rc==1（`Ended("suspended")` 挂起退出码契约）。
- 真实模型执行需 `ZHIPU_API_KEY`，本次环境不可用，**列为人工验证项**；语法 gate 已过。

## 明确不修（by-design）

- `externally_suspended` 吞掉 load 错误：挂起路径容错优先，保持现状。
- resume 不重验接管门：interrupt 即接管路径，不再二次校验。
- `accepted_generation` 记账：无消费者，维持不变。
- `prepare_sessions` 孤儿 Session：无泄漏窗口，不清理。
- 1000 轮父循环上限：防御性上限，保留。
- rewind 不重置 attempt：语义上保留失败计数。

## 回归

- `cargo test --workspace`：295 passed / 0 failed（含新增 todos 5 个测试文件、CLI 游标测试）。
- `cargo clippy --workspace --all-targets`：0 warning / 0 error。
- `python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py`：通过。

`transitions.rs` 现 775/800 行，逼近上限；后续新增断言一律放 `tests/`。
