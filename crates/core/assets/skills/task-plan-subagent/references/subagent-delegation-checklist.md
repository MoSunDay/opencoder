# Subagent Delegation Checklist

## 1. 拆分前检查

- 父 agent 是否只承担任务拆解、派发调度、结果回收和全局需求进展把控
- 是否已明确执行、集成、验证、验收和发布准备检查都必须派发给 subagent
- 总目标、非目标、上线标准和失败标准是否清楚
- 已确认问题、风险假设和外部 blocker 是否分开
- 当前 diff、相关模块、测试入口和发布约束是否已读取
- 是否存在必须先由用户确认的权限、数据、环境或发布时间决策
- 这些决策点是否已由父 agent 用 `question` 逐条对齐；无法提问时是否已写入 `assumptions:` 并标为对应 subagent 的验收前提

## 2. 子任务粒度

- 每个子任务是否只有一个清晰目标
- 是否能独立说明输入、输出、完成定义和验证方式
- 是否能在一轮 subagent 工作中稳定交付
- 是否避免了“顺便检查全部相关问题”的无边界任务
- 过大的子任务是否继续拆成探索、实现、验证、文档或集成任务

## 3. Ownership 与写集隔离

- 每个子任务是否有明确 owner 类型：explorer、worker、verifier 或 integration worker
- 并行 worker 的写入文件和模块是否互不重叠
- 如果必须修改同一文件，是否拆成顺序波次或交给单一 integration worker
- 每个 worker 是否知道禁止修改范围
- 是否明确不要 revert 他人改动、不要覆盖并行改动

## 4. 派发 Prompt 必备信息

每个 subagent prompt 至少包含：

- 背景与目标
- 负责范围和禁止范围
- 必读文件、可改文件、相关命令
- 仓库约束、敏感信息约束和危险操作护栏
- 交付物、验证命令、证据要求
- 失败时要报告的 blocker、日志、复现步骤或未完成原因

## 5. 回收与验收派发

- subagent 是否列出实际改动路径
- 是否给出执行过的验证命令和结果
- 是否说明未执行验证的原因、风险和补偿计划
- 是否存在越权修改、敏感信息、兜底掩盖错误或未说明风险
- 是否已为可交付子任务派发 verifier 或 acceptance subagent 做验收
- 是否需要后续 integration worker 处理冲突或拼接流程
- 是否避免父 agent 直接执行验证命令或自行给出验收通过结论

## 6. 最终集成与验证派发

- 所有子任务是否都有完成、失败或 blocker 状态
- 集成工作是否由 integration worker 完成，而不是父 agent 直接修改
- 集成后是否派发 verifier subagent 重新跑影响面级别验证
- 是否派发 verifier 或 release-check subagent 做 task-plan 级别遗漏复查
- 线上或生产等价验证方案是否仍然成立
- 最终交付是否基于 subagent 证据说明剩余风险、上线判断和后续 TODO

## Subagent Task Schema

每个子任务至少包含：

- `id`
- `owner 类型`
- `目标`
- `读范围`
- `写范围`
- `禁止范围`
- `依赖`
- `交付物`
- `验证方式`
- `回传格式`
- `失败处理`
- `当前状态`
