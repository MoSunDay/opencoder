# 大脑分层计划与运行协议

大脑仅支持 schema_version 4。计划保存 step（`nodes`）和连线（`edges`），每个 step 用一句话 `title` 描述任务并通过 `capability_id` 关联一个能力。计划保留名称、目标、默认工程输入、最多调度轮次；节点可配置尝试上限。层级由连线计算，不另存一份层表。

## 计划与能力

`POST /api/brain/plan-defs` 保存不可变版本，信封为 `{id, version, plan, changelog, created_at, author, tags, confidence}`；同 ID/版本不能覆盖，完全相同的重试幂等。`POST /api/brain/plan-defs/validate` 校验计划、能力可用性和嵌套引用。

```json
{
  "schema_version": 4,
  "title": "仓库检查",
  "objective": "收集证据后验证结果",
  "inputs": {"repo": "opencoder"},
  "nodes": [
    {"node_id": "collect", "title": "收集仓库证据", "capability_id": "builtin-operator"},
    {"node_id": "verify", "title": "验证收集的证据", "capability_id": "builtin-operator", "retry": {"max_attempts": 2}}
  ],
  "edges": [{"from": "collect", "to": "verify"}],
  "max_rounds": 32
}
```

能力 ID 以 `GET /api/brain/library` 返回为准。Agent、Team、DAG、TODO、Operator 使用现有执行接口；保存的计划版本也出现在能力库中，ID 为 `plan-{plan_id}@{version}`、kind 为 `brain`。选择该能力即执行固定版本的子计划。准入递归检查能力、引用环和深度（根深度 0，最多 3）；子计划绑定真实父 operation 身份，不能伪造父运行。

## 调度与屏障

计划按拓扑顺序从上到下切层，同层并行。每层只作一次模型调度决策，明确该层所有节点的输入绑定；层内运行终态经持久化 outbox 交付。全部成功后才越过屏障并激活下一轮决策。单个节点失败按 `retry.max_attempts` 重试（1–5，默认 2），每次尝试使用新的 operation/execution ID；达到上限立即使根运行失败并请求取消同层尚未结束的执行，迟到回执不会重新开启运行。

`run.layer` 记录已派发层，线上层号从 1 开始。等待期间仍显示当前层；越过屏障才进入下一层决策。最后一层成功后，以空节点上下文作完成决策，冻结结果摘要。暂停、恢复、取消沿用原命令；节点恢复依靠持久化投影、序号与确认记录，重复事件不产生重复派发。

输入绑定只能引用根输入、注册产物或成功祖先节点的 execution 输出 JSON pointer。运行输入覆盖计划默认输入。缺失能力、非法图或引用在准入时明确拒绝；运行中非法模型决策进入 blocked，可修正后恢复。限制：节点 1–256、每层最多 32、最多 32 层、调度预算 1–32。

## 接口与工作台

- `POST /api/brain/runs`：创建运行，推荐 `{schema_version:4, id, plan:{id,version}, inputs, node_id}`，也支持内联计划。相同 ID/意图重试复用回执，冲突返回 409。
- `GET /api/brain/runs/:id/layered`：计划、层级、能力元数据、操作索引和事件。
- `GET /api/brain/runs/:id/layered/rounds/:layer`：该层决策理由、证据 execution ID 与节点/尝试索引，不复制能力执行正文。
- `GET /api/brain/runs/:id/events-page` 与事件流：运行事件；`POST /api/brain/runs/:id/commands`：pause、resume、cancel。
- CLI：`brain plan-defs`、`brain library`、`brain runs create`、`brain runs layered`、`brain runs layered-round`。

工作台支持创建/编辑 step、选能力或保存计划、增删连线、浏览器草稿和固定版本启动。画布从上到下展示层级，点击节点或尝试复用现有 `ExecutionView` 查询真实能力明细；嵌套计划使用同一大脑运行组件。运行事件流与 3 秒轮询刷新索引，层明细按需读取。

旧决策树、playbook、v2/v3 调度内核、命令和页面已删除。旧数据不自动迁移或删除；生产清理由 `scripts/maintenance/brain_cleanup` 先生成精确清单，核准后离线备份和清理，能力库、鉴权和无关任务保留。
