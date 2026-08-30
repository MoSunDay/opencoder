Commit: 860831d22fad968737c366c93b4cf70fc1f4c010

# todos 模块

## 职责

`opencoder-todos` 执行预编译 `WorkflowSpec`。一个持久化 `workflow` Primary Session 只负责读取全局状态、选择可运行 TODO、决定并发批次和验收；每个 TODO 在自己的 `act` Primary Session 中完成完整闭环，可继续使用既有 subagent 工具。

## 边界

- `WorkflowSpec` 是启动前准备好的通用合同，不在运行时解释 UI Case 或自然语言步骤。
- Store 是唯一权威状态；文件目录只在 CLI `--debug` 下生成，不能反向覆盖 Store。
- 父 Session 不执行 TODO 工具，也不接收 MCP/registered CLI 指令；子 Session 不修改工作流投影。Rust 状态机校验双方结构化决定。
- 工具门禁读取执行 session 回调实时收集的 SessionEvent（同一回调经 `spawn_event_flusher` 同步落库供观测面回放），只接受声明工具的匹配参数与成功 ToolEnd，模型验收不能绕过失败门禁。

## 关键抽象

- `WorkflowSpec` / `TodoSpec`：目标、依赖 DAG、需求背景、聚焦指令、agent、最大尝试次数和验收合同。
- `WorkflowState` / `TodoState`：固定状态、attempt、当前位置、active/history Session、candidate、milestone、world epoch 与 incident。
- `ParentDecision`：`dispatch/mark_milestone/rewind/complete/fail/suspend`；dispatch 数量由父模型按 runnable 集合决定，没有固定工具调用或并发上限。
- `ContextMode::{New,Resume,Fork}`：父模型明确选择干净 Session、继续当前 Session，或从当前上下文派生新 Session。
- `persistence::commit`：以 generation 乐观并发控制，在同一 Store 提交工作流投影、TODO 投影和追加事件。

## 主流程

1. `validate_spec` 校验 ID、非空字段、依赖存在性和无环性。
2. 创建父 Workflow Session 和 Store 投影，进入 `running`。
3. 父 Session 读取当前全局状态并输出下一条结构化决策。
4. dispatch 前先创建 TODO Session 并原子写入 active Session 引用；一个批次中的 TODO 通过 `JoinSet` 并发执行。批结果逐项应用：执行错误经 `transitions::execution_failed` 落 NeedsRevision/Failed/Interrupted 并提交，兄弟 TODO 结果恒持久化，仅致命（转换/提交失败）错误挂起整个 run。
5. 子 Session 只收到当前 TODO、必要恢复摘要和工具合同，产出结构化 Candidate；解析器接受纯 JSON 或全文中唯一一个完整 JSON fence，并拒绝多个 fence 或未 fenced 的外围说明，工具门禁从事件记录独立计算。
6. 父 Session 验收 Candidate。通过后推进依赖；修订时选择 resume/fork；回退时推进 world epoch 并失效里程碑后的状态。
7. interrupt 或运行错误持久化为 `suspended`（terminal(Suspended) 把 Running/CandidateReady/Accepting 回滚为 `Interrupted` 并清空 stale candidate）；resume 先把中断中的 TODO 归约为可恢复状态，在 `workflow_resumed` 提交即持久化 `status=Running`（不额外 bump generation，维持 CAS 每提交 +1 不变量），再继续父决策循环；`resume` 拒绝 `status==Running` 的工作流防双驱动（错误信息含 `opencoder todos interrupt <id>` 接管指引）。interrupt 与并发提交撞 generation 冲突时有界重试（reload 后重判终态，上限 3 次），不再直接停车。
8. 容错硬化：非 Running 状态的迟到结果（Ok 与 Err 两侧对称，外部中断/同批 rewind 后）记日志丢弃不炸整轮；Blocked 项 attempt 耗尽归 `Failed` 而非永久跳过；dispatch 保留 `last_error`/`next_context_mode`（修订上下文不丢，仅清 candidate）；父 Session summary 不写入工作流状态；所有父决策与验收先在克隆状态上干跑校验（`validate_decision`/`validate_acceptance`），失败限次纠错重问（同 session，超限 suspend/bail），重复 `MarkMilestone` 幂等跳过；`validate_spec` 在提交期校验每个 todo 的 agent 可解析、is_primary、非 workflow、id 路径安全（`/`、`..`、`\0` 拒绝，防 `--debug` 投影逃逸），依赖环检测为迭代 DFS（深链不栈溢出）。

## 依赖与接口

- 依赖 `opencoder-session` 执行父/子 Primary Session，依赖 `opencoder-store` 保存状态，依赖 `opencoder-llm` 的可替换 `ChatStream`。
- CLI 入口见 [agents/cli](../cli/index.md)，持久化合同见 [agents/store](../store/index.md)。
- 用户能力见 [features/todos](../../features/todos/index.md)。

## 代表性验证

- `crates/todos/tests/runtime.rs`：单 TODO 闭环、父决定多 TODO 批次、debug 关闭不落目录、已有 debug 投影刷新、依赖环校验；`json_output` 单测覆盖结构化响应规范化。
- `crates/todos/tests/recovery.rs`：验收窗口崩溃 → runtime_error 挂起后 resume 自愈、父 fail/suspend 决定、`persistence::list` limit。
- `crates/todos/tests/interrupt.rs`：外部 interrupt 取消在飞 TODO 且可 resume、本地 Ctrl-C 单项标 Interrupted、终态拒绝 interrupt。
- `crates/todos/tests/transitions_guards.rs` / `late_results.rs` / `boundary_guards.rs` / `interrupt_retry.rs`：状态机守卫（max_attempts、Suspended 回滚、dispatch 保上下文）、迟到成功/失败结果丢弃、决策/验收干跑校验与纠错循环、重复里程碑幂等、resume 持久化 Running、interrupt 有界重试跨 generation 冲突。
- `crates/store/tests/todos_workflow.rs`：投影与事件原子提交、generation 冲突、v8 到 v9 迁移。
- `crates/cli/tests/todos_cli_parse.rs`：todos 命令和 debug 作用域。
