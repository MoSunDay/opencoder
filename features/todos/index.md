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
- crates/web/spa/src/todoEditor.jsx 与 src/todo/editor/ — SPA 模板编辑器（表单/画布/JSON 三态，画布可视化依赖）
- crates/todos/tests/ — 门禁、中断恢复与降级测试

## 边界

- 每个 TODO 独立 Primary Session；父会话不接执行工具
- required tool call 硬门禁：名称 + 参数子集 + 成功结果
- Store 是权威数据，debug 投影可重建
- 非 completed 终态退出非零；stdout 仅最终状态 JSON
- validate 拒绝含 /、..、\0 的 todo id 与依赖环
- SPA 画布编辑器的客户端校验是建议性镜像（crates/web/spa/src/todo/editor/specValidate.js），服务端 validate_spec 权威

## 相关

- [Agent 平台](../agent-platform/index.md)
- [todos 模块](../../agents/todos/index.md)
- [CLI](../../agents/local/index.md)
- [Store](../../agents/store/index.md)
