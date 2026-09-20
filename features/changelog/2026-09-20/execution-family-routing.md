Commit: 2866ae82c34999096efa8fe7e47003d114391120

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
- 工作区全量回归已完成：执行相关的 DAG、TODO、Agent、Team 专项均通过；首次全量运行中有一个并发脑调度用例偶发失败，单独复跑已通过。剩余失败集中在 `opencoder-session` 的 4 个 bash guard 兼容性用例，属于本次执行索引改动之外的现有门禁变更。
- 针对本次触及的 Store/Worker 目标执行 `cargo clippy --offline -p opencoder-store -p opencoder-worker --all-targets -- -D warnings`，无告警。
