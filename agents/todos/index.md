Commit: b98058ed96d82224f4070da893aecb653fafc6c8

# todos 模块

## 职责

`opencoder-todos` 执行预编译 `WorkflowSpec`。一个持久化 `workflow` Primary Session 只负责读取全局状态、选择可运行 TODO、决定并发批次和验收；每个 TODO 在自己的 `act` Primary Session 中完成完整闭环，可继续使用既有 subagent 工具。

## 边界

- `WorkflowSpec` 是启动前准备好的通用合同，不在运行时解释 UI Case 或自然语言步骤。
- Store 是唯一权威状态；文件目录只在 CLI `--debug` 下生成，不能反向覆盖 Store。
- 父 Session 不执行 TODO 工具；子 Session 不修改工作流投影。Rust 状态机校验双方结构化决定。
- 工具门禁读取持久化 SessionEvent，只接受声明工具的匹配参数与成功 ToolEnd，模型验收不能绕过失败门禁。

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
4. dispatch 前先创建 TODO Session 并原子写入 active Session 引用；一个批次中的 TODO 通过 `JoinSet` 并发执行。
5. 子 Session 只收到当前 TODO、必要恢复摘要和工具合同，产出结构化 Candidate；解析器只规范化纯 JSON 或单个 JSON fence，工具门禁从事件记录独立计算。
6. 父 Session 验收 Candidate。通过后推进依赖；修订时选择 resume/fork；回退时推进 world epoch 并失效里程碑后的状态。
7. interrupt 或运行错误持久化为 `suspended`；resume 先把中断中的 TODO 归约为可恢复状态，再继续父决策循环。

## 依赖与接口

- 依赖 `opencoder-session` 执行父/子 Primary Session，依赖 `opencoder-store` 保存状态，依赖 `opencoder-llm` 的可替换 `ChatStream`。
- CLI 入口见 [agents/cli](../cli/index.md)，持久化合同见 [agents/store](../store/index.md)。
- 用户能力见 [features/todos](../../features/todos/index.md)。

## 代表性验证

- `crates/todos/tests/runtime.rs`：单 TODO 闭环、父决定多 TODO 批次、debug 关闭不落目录、已有 debug 投影刷新、依赖环校验。
- `crates/store/tests/todos_workflow.rs`：投影与事件原子提交、generation 冲突、v8 到 v9 迁移。
- `crates/cli/tests/todos_cli_parse.rs`：todos 命令和 debug 作用域。
