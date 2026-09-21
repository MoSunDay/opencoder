Commit: 7177f7987d3c3cdc7ed17864a4ffcf347bf4d182

# 大脑调度工作台

版本化本体计划的创建、执行与运行查看工作台；细节以代码为准。

## 分层能力画布（v4）

`schema_version` 为 4（layered 契约）的运行，运行页展示「分层能力画布」：一列一层地绘制计划 DAG，标出层屏障进度、每个节点最新一次尝试的状态/重试次数与上游绑定，并可展开按层懒加载的层决策明细（层内涉及节点、尝试号、输入与摘要）、运行日志与能力清单；暂停 / 恢复 / 取消沿用运行命令。v3 运行仍走原工作台，未知的高版本给显式报错而非降级猜测。

- 读取 `GET /api/brain/runs/:id/layered`（视图）与 `GET /api/brain/runs/:id/layered/rounds/:round`（层明细）；非 v4 运行两者均 404，SPA 据此判定走 v3 工作台。
- `run.layer` 是已完成层数，正在决策的层恒为 `run.layer + 1`，线上层号从 1 起。
- 节点操作状态：`creating | running | done | error | cancelled`；运行阶段：`ready | deciding | waiting | paused | blocked | completed | failed | cancelled`。
- 现有限额：节点 ≤ 256、单层宽度 ≤ 32、深度 ≤ 3；`max_rounds` 缺省 32；节点尝试上限 1..=5（缺省 2），重试后同节点尝试号单调递增，呈现按最新一次尝试。
- CLI 读同一份视图：`opencoder-cli brain runs layered <id>` 与 `brain runs layered-round <id> <round>`；`brain runs create` 要求显式 `schema_version` 3 或 4，其余版本直接报错、不回落。
- 准入与运行面由根包 e2e [tests/brain_layered_e2e/](../../tests/brain_layered_e2e/main.rs) 覆盖（含真实 runc 画布跑到 `completed`）。

## 相关

- [运行协议](../../docs/brain-orchestration.md)
- [brain 模块](../../agents/brain/index.md)
- [control 模块](../../agents/control/index.md)
- [web 模块](../../agents/web/index.md)
