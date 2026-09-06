Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# team 模块

`opencoder-team` 是可嵌入的多 agent 讨论状态机，依赖 `Store`、`TeamDispatcher` 和 `TeamRunConfig`，不决定物理节点调度。

## 平台边界

[control](../control/index.md) 保存普通团队定义并选定一个 Node；[worker](../worker/index.md) 的本地 dispatcher 为 captain 和成员创建本地 agent 会话，角色和 agent 固定在执行快照中。成员 ID 表示团队角色，不等于机器 ID。

`system` 是唯一跨节点团队：协调记录在所属 Node，各成员通过 Server 的受约束 PeerBridge 调用各节点维护 agent。普通团队不使用旧 NodeDispatcher；旧 dispatcher 和 Web TeamHub 仅保留兼容接口。

权威讨论进度在 worker 的本地团队目录，`team_topic_runs` 只是运行台账。平台不通过 NFS 共享话题、消息或结果。

## 状态机

1. captain plan 选择本轮问题和参与成员，持久化 `plan.json`。
2. sub-turn 成员回答，captain 总结并判断是否对齐；未对齐按配置追问。
3. closing 决定结束或继续下一轮。结构化决策解析失败反馈纠正，超限以错误结束。

`cursor` 从持久化计划、结果、summary 和轮次账本推导恢复位置；`run_topic` 可恢复执行中或 error 话题，已完成的非 error 话题只幂等收束台账。

## 文件与接口

- `layout` 纯路径校验：团队名小写/数字/连字符；topic 支持 ULID 及平台 `team-` / `system-` 执行 ID。
- `fs_store` 集中目录 IO，临时文件加 rename 原子替换，读取带尺寸限制。
- `prompts` 构造计划、回答、对齐、总结和收尾提示；`decide` 校验队长 JSON 决策。
- `runtime`、`runtime/stages` 驱动状态机，`terminal` 统一写终态。
- `TeamDispatcher::ask` 返回成员回答；worker 汇总成员执行错误，不能因队长宣告完成而吞掉成员失败。

目录形状为 `<team_root>/<team>/<topic>/team.json` 及逐轮 plan、result、summary 文件。运行明细经 Node 查询，业务规则见 [Agent 平台](../../features/agent-platform/index.md)。
