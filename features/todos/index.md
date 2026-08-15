Commit: 1ba8f4264210ee9212d2158b2d928ef4b2411477

# 持久化 TODO 工作流

`opencoder todos` 面向已经拆解完成的 `WorkflowSpec`。调用方在启动前提供目标、TODO DAG、每项需求背景、聚焦指令和验收工具合同；运行时不重新理解或改写原任务。

用户可见规则：

- `validate --file` 只校验合同，不创建 Session、不调用模型。
- `run --file` 创建唯一运行 ID。一个父 Workflow Session 管理全局进度，每个 TODO 对应独立 Primary Session；该 Session 内可进行任意必要模型轮次、工具调用及 subagent 调度。
- 父工作流按依赖和当前状态决定一次 dispatch 一个或多个 TODO；“聚焦一个 TODO”不等于“每轮只能调用一个工具”。
- 父 workflow 不接收执行工具或 registered CLI 指令。移动 UI TODO 可显式激活内置 `$fk-cli`，并由独立 TODO Session 仅通过 `bash` 调用预编译的精确 `fk-session --args` 命令；无需配置 FK MCP。
- TODO 固定记录 status、attempt、active/history Session、Candidate、恢复摘要和验收结果；Workflow 固定记录 generation、当前位置集合、milestone、world epoch、incident 与终态原因。
- required tool call 是硬门禁。名称、声明参数子集和成功结果必须同时匹配；父模型不能接受未满足门禁的 Candidate。
- `interrupt` 持久化挂起状态；`resume` 从 Store 对运行中 TODO 做中断归约后继续。父工作流可选择 resume、fork 或回退到 milestone。
- Store 永远是权威数据。`--debug` 为 `run/resume` 生成 `<data-dir>/todos/<run-id>/task-info`、`process` 和 `sessions` 的可恢复索引投影；后续 `interrupt` 会同步已经存在的投影，但不会为非 debug 运行新建目录。
- 结构化模型输出接受纯 JSON 或全文中唯一一个完整 JSON fence；多个 fence、未 fenced 的说明加 JSON 或无法唯一定位的输出仍按合同错误失败。
- 非 completed 终态让 `run/resume` 返回非零，合同错误、模型 JSON 错误和持久化冲突不会降级为猜测性继续。

相关逻辑：[todos 模块](../../agents/todos/index.md)、[CLI](../../agents/cli/index.md)、[Store](../../agents/store/index.md)。
