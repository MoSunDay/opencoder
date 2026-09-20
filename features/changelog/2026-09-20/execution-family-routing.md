# DAG、TODO、Agent 与 Team 执行索引和传参

## 变更

- 节点执行索引增加独立的执行会话查询视图，DAG Agent 步骤可通过带类型的执行引用读取。
- Team 的 `member-*` 会话继续作为 Agent 执行索引暴露，成员能力前缀和任务输入仍由节点本地执行。
- DAG Agent 步骤与 Team 成员不会进入聊天会话列表；详情仍通过执行 ID 查询。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| DAG 输入参数 | `dispatch_input_args_append_to_the_wasm_command_line` | `tests/dag_e2e/input_args.rs` |
| DAG Agent 步骤执行和详情 | `dag_spec_dispatch_runs_wasm_and_agent_steps_to_done` | `tests/dag_e2e/flow.rs` |
| Web DAG Agent 步骤会话 | `claimed_run_executes_and_converges_done_on_the_server` | `crates/web/tests/dag_e2e_flow.rs` |
| Team 成员执行与能力传参 | `team_members_execute_locally_with_capability_prefixes` | `crates/worker/tests/workloads.rs` |
| Team 多轮执行 | `team_multiround_consensus_runs_alignment_subturn_and_next_round_hint` | `crates/worker/tests/platform/team_multiround_consensus.rs` |
| TODO 工作流执行和子任务传参 | `todo_template_runs_to_completed_with_passed_item` | `tests/todos_e2e/flow.rs` |
| Agent 执行和输出传参 | `agent_session_runs_prompt_and_exposes_output` | `tests/operator_e2e/agent_session.rs` |
| 控制面输入透传 | `dispatch_passes_input_through_to_the_assignment` | `crates/control/tests/e2e/dag_dispatch_extra.rs` |

- 上述专项回归均通过。
- 工作区全量回归已发起；当前工作区存在其他并行改动和编译任务，需在共享编译锁释放后复核最终结果。
