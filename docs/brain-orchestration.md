# 大脑里程碑计划与运行协议

新计划和运行使用 `schema_version: 7`。`layers` 按顺序定义里程碑，每层有独立的名称、目标和达成标准；`nodes` 是层内并行执行项，每个执行项通过 `layer_id` 归属一层，并且只绑定一个 `capability_id`。`transitions` 是层到层的有向连线，条件文本为大脑的决策依据。首轮从第一层开始；前进只能到紧邻下一层，回退可连接本层或先前已执行层。末层即使有回退线，达标后仍可完成。

## 配置与能力

在画布配置大容器里程碑、并行执行节点和层间扭转条件，再通过表单填写计划名称、整体目标、工程输入与轮次预算。节点坐标只用于布局，不定义调度顺序。浏览器草稿保留画布与表单内容；保存创建不可变版本，运行须另行启动。

`POST /api/brain/plan-defs` 保存 `{id, version, plan, changelog, created_at, author, tags, confidence}`，同 ID/版本不可覆盖，相同提交幂等。`POST /api/brain/plan-defs/validate` 检查结构、能力及嵌套引用。

```json
{
  "schema_version": 7,
  "title": "开发与验证",
  "objective": "交付经过验证的变更",
  "inputs": {"repo": "opencoder"},
  "max_rounds": 5,
  "layers": [
    {"layer_id":"coding","title":"Coding","objective":"实现变更","success_criteria":"代码与证据达到交付标准"},
    {"layer_id":"testing","title":"测试","objective":"验证变更","success_criteria":"测试与审查通过"}
  ],
  "nodes": [
    {"node_id":"code","layer_id":"coding","title":"编码","objective":"实现或整改变更","capability_id":"builtin-agent-act"},
    {"node_id":"review","layer_id":"coding","title":"并行审查","objective":"审查变更","capability_id":"builtin-agent-act"},
    {"node_id":"test","layer_id":"testing","title":"运行测试","objective":"返回测试证据","capability_id":"builtin-operator"}
  ],
  "transitions": [
    {"from":"coding","to":"testing","condition":"实现达到当前层标准"},
    {"from":"testing","to":"coding","condition":"测试发现需要整改的问题"}
  ]
}
```

能力 ID、输入输出及必填参数以 `GET /api/brain/library` 为准。运行准入冻结所引用能力的定义；每次派发必须覆盖目标层全部执行节点，每个节点调用其绑定能力一次。Agent、Team、DAG、TODO、Operator 和 Brain 复用已有执行接口。已保存的 schema 7 计划版本以 `plan-{plan_id}@{version}`、kind `brain` 出现在能力库；嵌套引用递归验证，根深度为 0，最多深度 3，子计划必须绑定真实父 operation。

## 决策、回退与轮次

所有同层执行进入终态后，大脑才评估当前层。`assessments` 必须只含当前 `layer_id`，说明达成标准是否满足及证据；首次派发的评估为空。前进或完成要求达标且执行成功。下一次派发只能选择当前层的一条出边；回退或本层重试需要非空 `reflection`，并开启下一轮，令目标层及后续层的早先成果失效。历史执行和派发理由保留。没有节点自动重试或失败时自动取消同层任务。

首轮为第 1 轮，正常前进不增加轮数。默认预算 5 轮，允许 1–32 轮；预算耗尽进入 `blocked`，不创建新执行。暂停或阻塞时可用 `set_round_budget` 增加预算，再 `resume`。每次派发增加 `activation`，operation/execution ID 包含激活身份，迟到和重复回执不推进当前决策。

输入绑定支持根输入、注册产物、终态 execution 输出的 JSON pointer，以及 `{kind:"value",value:...}` 的具体任务输入。能力只接收本节点任务与绑定输入；全局方法论、反思和转移决策留在大脑。模型上下文保留各层最近一次激活及有界摘要，完整历史留在日志中。非法决策最多自动纠正两次，仍不合法则阻塞；恢复时上次校验错误会反馈给模型。

限制：1–32 层、总共 1–256 个执行节点、每层 1–32 个节点、每节点一个能力。新计划不接受旧节点重试策略和旧 `edges` 连线。

## 查询与历史

- `POST /api/brain/runs`：`{schema_version:7,id,plan:{id,version},inputs,node_id}`，也支持内联计划。相同 ID/意图重放回执，冲突返回 409。
- `GET /api/brain/runs/:id/layered`：冻结计划、层级、运行、操作索引、能力元数据和事件。
- `GET /api/brain/runs/:id/layered/rounds/:layer?activation=:id`：指定层激活的 `visit`、历史 `visits`、层里程碑评估及各节点执行索引；省略 activation 返回最新一次。路径中的 `rounds` 是历史接口名称，参数仍为层号。
- `GET /api/brain/runs/:id/events-page` 与事件流：运行日志。
- `POST /api/brain/runs/:id/commands`：暂停、恢复、取消，或 `set_round_budget`。

工作台按轮次与激活展示理由、输入绑定、执行类型和 ID，点击节点复用现有 `ExecutionView` 按 ID 查询明细。历史 schema 4/5/6 计划与已结束运行只读；编辑旧计划显式创建 schema 7 新版本，不改写旧数据。升级和回滚前检查未结束运行，不支持的运行须在原 Runtime 收敛；历史运行仍由所属 Runtime 提供查询。
