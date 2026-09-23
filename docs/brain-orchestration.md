# 大脑里程碑计划与运行协议

新计划和新运行使用 `schema_version: 6`。节点是里程碑，保存 `title`、`objective`、`success_criteria`、`layer` 和允许调用的 `capability_ids`。层号从 1 连续递增，同层里程碑并行；正常推进按层顺序进行。回退由大脑在已执行层中选择，允许形成环；新计划的 `edges` 为空。

## 配置与能力

先在画布添加、移动和配置里程碑，挂载能力，设置同层并行和顺序层级；再进入独立表单填写计划名称、整体目标、工程参数及轮次预算。浏览器保留画布和表单草稿，保存不可变计划版本后另行启动执行。节点坐标只用于浏览器布局，调度只读明确的层号。

`POST /api/brain/plan-defs` 保存 `{id, version, plan, changelog, created_at, author, tags, confidence}`，同 ID/版本不可覆盖，相同提交幂等。`POST /api/brain/plan-defs/validate` 检查计划、能力及嵌套引用。

```json
{
  "schema_version": 6,
  "title": "开发与验证",
  "objective": "交付经过验证的变更",
  "inputs": {"repo": "opencoder"},
  "max_rounds": 5,
  "nodes": [
    {"node_id":"code","title":"Coding","layer":1,"objective":"实现或整改变更","success_criteria":"实现完成并提供验证证据","capability_ids":["builtin-agent-act"]},
    {"node_id":"test","title":"测试","layer":2,"objective":"验证需求和变更","success_criteria":"测试通过","capability_ids":["builtin-operator"]}
  ],
  "edges": []
}
```

能力 ID 和输入输出契约以 `GET /api/brain/library` 为准。运行准入冻结所引用能力的定义；调度从各里程碑允许的能力集中选择一项或多项。Agent、Team、DAG、TODO、Operator 复用现有执行接口；已保存的 schema 6 计划版本以 `plan-{plan_id}@{version}`、kind `brain` 出现在能力库中。嵌套引用在准入时递归验证，根深度为 0，最多深度 3，子计划必须绑定真实父 operation。

## 决策、回退与轮次

每次派发必须覆盖目标层的全部里程碑，每个里程碑至少选一个能力。所有选中执行都进入终态（包括失败、取消）后才触发一次大脑决策；部分完成只更新记录。没有节点自动重试或失败时自动取消同层任务。

大脑通过 `assessments` 对当前层每个里程碑作业务判定，说明达成标准是否满足及依据。前进只能到下一层，且当前里程碑全部达标；回退可选择当前或更早的已执行层，并提供非空 `reflection`。回退使目标层及其后续层此前的达成记录失效，保留前缀成果和全部历史执行。最终层仍可反思回退，只有全部有效层通过才能完成运行。

首轮为第 1 轮，正常前进不增加轮数，每次回退增加一轮。默认预算 5 轮，允许 1–32 轮；预算耗尽进入 `blocked`，不创建新执行。暂停或阻塞时可通过 `set_round_budget` 增加预算，再 `resume`。每次派发增加 `activation`，operation/execution ID 包含本次激活身份，同一里程碑重跑不会覆盖旧执行。旧激活及重复终态回执不会推进当前决策。

输入绑定支持根输入、注册产物、终态 execution 输出的 JSON pointer，以及 `{kind:"value", value:...}` 形式的具体任务输入。执行能力只接收所在里程碑和显式绑定输入；根计划和全局反思留在大脑，整改要求由大脑转换为该能力的具体任务输入。失败结果可用于诊断和整改；历史结果不会自动成为有效成果。计划默认输入可由运行输入覆盖。模型上下文只保留各层最近一次激活及有界结果摘要，能力契约去重，完整历史仍保留在日志中。非法决策最多自动纠正两次；纠错期间不派发能力，仍不合法则明确进入 `blocked`。决策输入显式列出本次必须评估的节点 ID；恢复时保留上次校验错误作为模型反馈，有效决策通过后清除。

限制：1–256 个里程碑、每层最多 32 个里程碑、最多 32 层、每个里程碑允许 1–32 个能力、单次最多派发 256 项执行。新计划不接受旧的节点重试策略。

## 查询与工作台

- `POST /api/brain/runs`：`{schema_version:6, id, plan:{id,version}, inputs, node_id}`，也支持内联计划。相同 ID/意图重放回执，冲突返回 409。
- `GET /api/brain/runs/:id/layered`：计划、层级、运行、操作索引、能力元数据和事件。
- `GET /api/brain/runs/:id/layered/rounds/:layer?activation=:id`：指定层激活的 `visit`、该层全部 `visits`、里程碑判定及各能力执行索引；省略 activation 返回最新一次。路径中的 rounds 是历史接口名称，参数仍为层号。
- `GET /api/brain/runs/:id/events-page` 与事件流：运行日志。
- `POST /api/brain/runs/:id/commands`：`{action:"pause"|"resume"|"cancel"}`，或 `{action:"set_round_budget", input:{max_rounds:8}}`。

工作台按轮次、层激活、里程碑和能力显示调度理由、输入绑定、反思及执行类型/ID。点击执行复用六类现有 `ExecutionView` 面板，正文按 ID 向所属执行查询。层激活历史取自不可变的 `layer_started` 事件，不会被后来回执或重跑覆盖。

## 历史版本与升级

schema 4/5 计划和已结束运行保持可读；编辑旧计划是显式创建 schema 6 新版本，需要补齐里程碑目标和达成标准。旧回退连线转换时被移除；旧 DAG 连线不会被自动解释为回退线。旧数据不删除，不自动改写，也不能直接创建新的 schema 4 运行。

发布包声明 `brain_schema_version`。升级和回滚前检查未结束运行；不支持的运行必须先在原 Runtime 收敛，不能由新内核接管。历史运行仍由其所属 Runtime 提供查询；新 Worker 读取本地已结束 schema 4/5 日志时只开放查询。
