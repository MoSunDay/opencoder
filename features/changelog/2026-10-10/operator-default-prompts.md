Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# Operator 默认提示词与 skill 目录

Operator 的默认规则独立于首条用户消息，明确当前任务的目标、范围、交付与完成条件，并引导按需读取 skill。技能目录来自当前会话，Agent 私有技能优先于本次执行冻结的技能池；不一次注入全部技能正文。

OpenCoder 将规则加入每轮系统提示词和上下文预算；Codex 每轮携带相同规则。会话类型沿用已有持久化字段，未新增表或环境变量。后台会话显式继承执行技能根，较大的会话 future 使用堆分配，避免嵌套作用域触发线程栈溢出。

相关索引：[session](../../../agents/session/index.md)、[worker](../../../agents/worker/index.md)、[web](../../../agents/web/index.md)、[用户行为](../../agent-platform/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 单一任务规则、目录顺序与去重 | `operator_lists_ordered_unique_paths_and_task_contract` | [operator.rs](../../../crates/session/src/prompt/operator.rs) |
| 无目录及非 Operator 边界 | `missing_roots_do_not_invent_a_home_directory`、`other_execution_kinds_have_no_operator_instructions` | [operator.rs](../../../crates/session/src/prompt/operator.rs) |
| 压缩、清空上下文与恢复 | `operator_defaults_survive_compaction_context_clear_and_resume` | [operator_prompt.rs](../../../crates/session/tests/operator_prompt.rs) |
| 其他会话类型不注入规则 | `other_session_kinds_do_not_receive_operator_defaults` | [operator_prompt.rs](../../../crates/session/tests/operator_prompt.rs) |
| Codex 私有技能、恢复及派生会话 | `codex_operator_defaults_include_private_skills_on_start_resume_and_fork` | [harness_operator.rs](../../../crates/session/tests/harness_operator.rs) |
| Worker 系统指令与用户原文 | `operator_executions_run_the_host_process_agent_loop` | [workloads.rs](../../../crates/worker/tests/platform/workloads.rs) |
| 真实服务首次及后续消息、冻结技能路径 | `flow::operator_session_runs_prompt_to_idle` | [flow.rs](../../../tests/operator_e2e/flow.rs) |

验证结果：

- `cargo test -p opencoder-session --test operator_prompt --test harness_operator`：3 passed，0 failed。
- session 单测程序运行 `prompt::operator::tests`：3 passed，0 failed。
- 准备版本一致的原生测试镜像后，Worker 的 `workloads::operator_executions_run_the_host_process_agent_loop --exact`：1 passed，0 failed。
- 重新构建后的 `operator_e2e` 测试程序运行 `flow::operator_session_runs_prompt_to_idle --exact`：1 passed，0 failed。
- `cargo build --workspace -j 4`：通过。
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings`：通过，零警告。
- Worker `platform` 测试组扩大验证：4 passed，5 failed；失败均为 DAG 或多轮 Team 等待完成超时，Operator 用例通过。日志：`/tmp/opencoder-operator-worker-platform.log`。
- `cargo test --workspace -j 8 -- --test-threads=4` 已执行；同机磁盘负载较高，编译持续 68 分钟仍未进入测试阶段，已终止本次运行，未取得全量通过结果。日志：`/tmp/opencoder-operator-regression-verified.log`。
