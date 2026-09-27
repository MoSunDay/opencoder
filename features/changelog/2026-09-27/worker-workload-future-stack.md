Commit: 24a1081aff7fd8591f860723bae5eb787edf68e9

# Worker 工作负载 future 栈空间

`todos_e2e` 在默认 Tokio worker 栈上触发 Agent 进程栈溢出，模型请求无法开始；增大线程栈后同一场景通过。Worker 现在把各工作负载的异步分支收束为堆上的 `WorkloadFuture`，避免分发函数在 worker 栈中保留大型分支 future，同时保持原有执行结果和错误传播契约。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| TODO 完整执行 | `todo_template_runs_to_completed_with_passed_item` | `tests/todos_e2e/flow.rs` |
| 子模型失败与暂停 | `child_model_failure_suspends_workflow_with_failed_todo` | `tests/todos_e2e/lifecycle.rs` |
| 中断恢复 | `interrupt_survives_node_restart_and_resumes_to_done` | `tests/todos_e2e/lifecycle.rs` |

- 默认线程栈专项回归：`cargo test --test todos_e2e` → 3 passed / 0 failed。
- Worker 单测：`cargo test -p opencoder-worker --lib` → 105 passed / 0 failed / 1 ignored。
- Worker Clippy：`cargo clippy -p opencoder-worker --all-targets -- -D warnings` → 零警告。
