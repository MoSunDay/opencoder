Commit: 3908926718bbcaec47614743e1228a91c40c4362 (working-tree)

# 三处陈旧测试期望对齐现行契约（全量回归解锁）

全量回归发现三个在干净 HEAD 即失败、与并发特性无关的陈旧断言，逐一按现行 LOCKED 契约修正：

1. **web `dag_e2e_flow`**：live execution logs（104c6b26）使 SSE 投影在 `step_started` 与 `step_done` 之间多出 `step_log`（text_delta）帧；期望序列补入该帧并新增 step_log payload（step/event/text）断言。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| e2e SSE 投影含 step_log 帧 | `claimed_run_executes_and_converges_done_on_the_server` | `crates/web/tests/dag_e2e_flow.rs` |
| runner 步骤双路径拒绝 | `registered_runner_is_rejected_by_dag_definition_and_inline_dispatch` | `crates/worker/tests/runner_dispatch.rs` |

- 全量回归：`cargo test --workspace --no-fail-fast` → 5185 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
