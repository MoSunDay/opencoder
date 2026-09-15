Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# 持久化 TODO 工作流 — 父会话调度验收、独立 TODO 执行

## 关键路径

- crates/todos/src/types.rs — WorkflowSpec 与 required_tool_calls
- crates/todos/src/parent.rs — 父 Workflow Session 调度与验收
- crates/todos/src/runner.rs — 运行时与 --debug 投影
- crates/todos/src/persistence.rs — Store 持久化与 debug_dump
- crates/todos/src/transitions.rs — 状态机与里程碑幂等
- crates/todos/src/json_output.rs — 最终状态 JSON 输出
- crates/local/src/todos_cmd.rs — validate/run/resume/interrupt
- crates/web/src/api_todo_runs.rs — 平台 TODO 运行接口
- crates/web/src/api_todo_templates.rs — TODO 模板接口
- crates/web/spa/src/todoEditor.jsx 与 src/todo/editor/ — SPA 模板编辑器（默认画布，表单/JSON 共享草稿，metadata 与依赖引用保留）；宿主是 todoPanel 的 100% 宽右侧抽屉，非整页替换
- crates/web/spa/src/todoRunsPanel.jsx 与 src/todo/runCanvas.jsx — 运行工作台：调度画布、节点 Review、历史尝试与任意节点重跑
- crates/todos/src/execution.rs — TODO env 生效链：dispatch 盖章的 `metadata.env_vars` 并入子会话 `env_passthrough`，经 `ToolContext::extra_env` 抵达 bash/harness 进程
- crates/todos/tests/env_passthrough.rs — env_vars 抵达 bash 进程的生效证明
- crates/todos/tests/ — 门禁、中断恢复与降级测试

## 边界

- 每个 TODO 独立 Primary Session；父会话不接执行工具
- required tool call 硬门禁：名称 + 参数子集 + 成功结果
- Store 是权威数据，debug 投影可重建
- 非 completed 终态退出非零；stdout 仅最终状态 JSON
- validate 拒绝含 /、..、\0 的 todo id 与依赖环
- TODO env `env_vars` 键必须匹配环境变量名、值必须字符串；dispatch 盖章与 env 保存双重 fail-fast
- 节点侧 OpenCoder Env 配置集（/api/envs）已删除，TODO env 是唯一环境体系
- SPA 画布编辑器的客户端校验是建议性镜像（crates/web/spa/src/todo/editor/specValidate.js），服务端 validate_spec 权威

- `crates/todos/src/review/` 统一派发上下文、精简父 Agent 调度输入与重跑影响范围。
- `crates/worker/src/operations/todo/` 提供一致性快照、分段 Review 和持久化重跑受理恢复；旧执行退出后才重新排队。
- 重跑保留当前文件、外部操作结果、上游及独立分支的通过结果，并归档目标与下游旧轮次。

## 相关

- [TODO 工作台操作](../../docs/todo-workbench.md)
- [Agent 平台](../agent-platform/index.md)
- [todos 模块](../../agents/todos/index.md)
- [CLI](../../agents/local/index.md)
- [Store](../../agents/store/index.md)
