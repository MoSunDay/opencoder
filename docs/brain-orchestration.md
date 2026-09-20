# 大脑调度计划与运行协议

大脑有两条协议。v2 使用 `input → 实例 → output → 路由 → 下一实例的 input` 的不可变图；v3 使用工程输入、能力目录和轮次终态事件的轻量调度。v2 计划仍按原校验器和执行内核运行，v3 不把完整 DAG 或子执行正文复制到脑状态。

## 事件驱动调度 v3

v3 请求必须明确 `schema_version: 3`，根输入使用命名 JSON 输入，`repo`、`commit`、`branch` 等工程信息只是普通输入。每轮模型只能返回严格的 `Dispatch`、`Complete` 或 `Fail`：能力必须来自目录，输入绑定只能引用根输入、成功执行的 `execution_id` 输出路径或已有产物引用。缺少目录描述、非法引用、无成功证据完成或超过轮次上限都会进入 `blocked`。

运行投影只保存 `run`、`operation` 和 `event` 的索引、状态、序号、引用及有限摘要。创建一轮时 control 通过统一 gateway 调用真实 Agent、Team、DAG、TODO 或 Operator；调度器随后停止，只有节点持久化终态并经 outbox 确认的事件才能唤醒下一次判断。当前轮次全部成功才越过屏障；任一失败终态立即取消兄弟操作并终止运行，迟到事件只记录。执行详情、消息、DAG 步骤和产物正文通过 `GET /api/executions/{execution_id}` 及所属节点查询。

v3 入口为 `POST /api/brain/runs`、`GET /api/brain/runs/:id`、`GET /api/brain/runs/:id/events-page`、`GET /api/brain/runs/:id/rounds/:round` 和 `POST /api/brain/runs/:id/commands`；CLI 的 `brain runs` 支持创建、最小快照、轮次、事件及 pause/resume/cancel。v2 数据不迁移、不删除，旧运行保持只读兼容。

## 可复用调度计划与工作台

`/api/brain/plan-defs` 的版本 envelope 保持 `{id, version, plan, changelog, created_at, ...}`。新增 v3 计划内容：

```json
{"schema_version":3,"title":"修复并复测","objective":"完成修复并提交测试依据","inputs":{"repo":"example"},"capability_ids":["builtin-agent-act"],"max_rounds":32}
```

计划要求非空能力范围，保存前校验目录；版本仍沿用现有不可覆盖、相同内容幂等及顺序递增规则。列表提供 `schema_version` 区分历史图计划和调度计划。查询历史版本、版本比较保持原 URL 和 envelope。

引用版本启动：

```json
{"schema_version":3,"id":"brain-review-001","node_id":"node-example","plan":{"id":"review-plan","version":1},"inputs":{"repo":"override"}}
```

服务端从版本解析目标、能力范围和轮次上限，运行输入按名称覆盖默认输入；不允许同时传入目标等计划字段覆盖版本。根 assignment 保存来源版本、解析后的 scheduler_request、原始幂等意图和能力范围元数据。回执为 `202 {schema_version:3, run_id, execution}`，重试相同意图复用运行，冲突意图返回 409。既有直接提交 v3 请求的方式继续支持。

`GET /api/brain/runs/:id/view` 展示来源计划、所选能力、轮次 operation 索引及持久化决策摘要。每项 operation 增补 `execution_created` 和派发时能力元数据；预分配 ID 不代表子执行已创建。`rounds/:round` 返回相同的单轮投影。详情仍按 execution_id 查询，不将消息或产物复制到大脑投影。

计划画布固定显示核心调度循环，能力库连接调度环节。运行页按轮展开，在同页复用五类执行组件；切换 execution_id 会重新挂载明细和订阅。新计划草稿与旧版缓存隔离，历史图计划保留只读页面。

## 历史 v2 图契约

以下记录历史固定图协议，供历史版本、运行及兼容路径核对；新工作台创建与执行使用上面的 v3 调度计划。

## 四类概念

- `inputs`：按名称索引的输入，包含 `description`、`schema`、`required`、`source: {kind: external|routed}`。实例通过 `inputs: [名称]` 引用。外部文档值是 `{name: "需求说明", markdown: "# 正文"}`。
- `instances`：注册能力的一次引用，包含 `id`、`description`、`capability_id`、`action`、输入及输出名称、资源和 `max_visits`（默认 20，可设 1–100）。支持 Agent、DAG、Team、TODO、Operator。
- `outputs`：按名称索引的一句话说明。每个输出只有一个实例拥有，实际内容可以是完整文档或结构化数据；结构化回执上限 256 KiB，较大内容通过产物引用提供。
- `routes`：包含 `id`、`description`、连接的 `outputs`、候选 `targets` 和明确 `exits`。目标的 `bindings` 固定映射目标 input 名称到已连接 output 名称。多个实例的 output 接到同一路由即汇合；路由可选择多个候选实例并行执行，也可回到已有实例开始新轮次。

完整可编辑示例：[修复与复测循环](../examples/brain/repair-loop.json)。`entry` 显式列出初次激活的实例。每个实例的输出由一个路由统一判断；不能用多个独立路由重复消费同一实例的完成回执。

计划保存：`POST /api/brain/plan-defs`，请求为 `{id, version, plan, changelog, created_at, author, tags?, confidence?}`。同 ID/版本不可覆盖，相同内容重试幂等。发布时检查能力注册身份与描述，固定定义、执行配置和资源摘要。能力缺失或快照不符会明确报错。

运行提交：

```json
{
  "id": "brain-review-001",
  "node_id": "node-example",
  "mode": "fixed",
  "objective": "完成修复并提供验证依据",
  "plan": {"id": "repair-loop", "version": 1},
  "inputs": {"document": {"name": "需求说明", "markdown": "# 问题\n修复失败的检查"}}
}
```

动态模式设 `mode: dynamic`，省略 `plan`，可传 `references: [{id, version}]`。服务端取得注册目录后生成并发布同一格式的计划；不会创建临时能力或默认改派通用 Agent。

## 输出和局部路由

每种能力通过同一个接口提供具名输出：

```json
{
  "verification": {
    "content": "本轮测试通过；修复提交及测试报告见产物",
    "completion": {"passed": true, "evidence": ["交付内容已生成"]},
    "verification": {"passed": true, "evidence": ["本轮测试报告：全部通过"]},
    "artifacts": []
  }
}
```

完成与验证分别记录。`passed: null` 或缺失依据都为未知，进程结束不能推导为验证通过。Agent/Operator 读取最后回答，Team 读取 final_summary，DAG/TODO 通过固定输出投影适配原生产物；返回内容必须符合声明的具名输出接口。

DAG/TODO 的原生结果按内部步骤名组织。实例的 `action.output_pointer` 固定指定提供统一输出的结果位置，例如 DAG 的 `/review` 或 TODO 的 `/t1`；该位置必须返回上述具名 JSON 对象。适配仅提取结果，不读取能力内部状态推断业务通过。

路由模型只接收本次连接的 output 记录、路由语义、相邻候选实例的输入说明和声明的结束条件。完整计划、根目标、无关节点输出和能力内部状态不进入路由请求。路由回执：

```json
{"receipt":"route-...","reason":"本轮复测仍有两个问题，继续修复","selected":["fix"],"exit":null,"blocked":null}
```

`selected` 可选一个或多个相邻实例，或者 `exit` 指向声明的出口。越界选择、空选择、非法 JSON、缺失必要输出及未知交付结论均持久化阻塞原因，不重写计划或暗中降级。

## 汇合、循环与恢复

- 汇合等待仍可能到达该路由的已激活分支；未选择的分支不参与等待。输入引用固定到实际执行轮次，禁止按实例名称读取“最近一次”输出。
- 每次并行分流以路由回执建立因果作用域，汇合消费同一作用域的分支。并发进入同一子流程的两组输出分别汇合，通知乱序也不会串轮。
- 回流创建新的 `~visit-NNNN` 执行轮次，旧输出、因果父节点、输入引用及路由判断全部保留。达到访问上限记为 blocked。
- 下游派发的 `parameters` 承接映射的实际内容，`source_outputs` 按 input 名称附带同轮次 output 记录及产物引用；不会混入其他上游历史。
- 路由上下文先持久化；判断、输入引用和 Prepared 执行回执在派发前提交。重复通知、乱序回执和恢复重放沿用原动作 ID。
- 暂停阻止新派发；取消等待在途子执行明确结束。共享资源继续使用全局读写互斥与明确终态释放。
- 根运行只有在命中声明出口、满足出口交付要求且全部激活分支收敛时 completed。出口可要求 `require_completed` 和 `require_verified`，必须有相应 output 依据。
- `graph.outputs` 保存轮次与内容，`graph.visits` 保存输入引用和因果父节点，`graph.routes` 保存读集和选择理由，`graph.tokens` 表示当前分支。事件同步记录新增输出与路由回执。

## 查询与迁移

版本、快照、实例分页、动作、事件及控制入口保持 `/api/brain/plan-defs` 与 `/api/brain/runs`。CLI 使用 `brain plan-defs`、`brain runs`、`brain library`。工作台可编辑四类概念、具名文档、多输出、汇合和回流，并查看完成与验证依据。

历史 v1 计划、决策树、Playbook 和运行记录继续只读查询；旧写入和执行入口返回 migration required，包括旧 Project Brain/Playbook 路由和本地入口。既有数据不删除、不改写、不静默转换。显式重建为 v2 后通过统一运行入口执行。

Fleet 协议为 10；根运行请求带 `schema_version: 2`。旧节点不得受理新契约。发布工具在启动候选服务及切换前只读扫描旧运行；节点启动在恢复写入前再次检查。存在非终态旧运行时拒绝升级并列出执行 ID，须让所属旧运行正常收敛。协议不同的版本仍禁止滚动重叠，必须走既有维护迁移流程；本次变更不执行生产切换。
