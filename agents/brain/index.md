Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# brain 模块

注册能力目录、不可变 v2 计划与事件驱动 v3 调度器。v2 计划以 input、实例、output、路由组织；v3 只保存轮次、能力执行索引和调度事件，能力自身负责执行和验证。

## 执行边界

- `graph/validate.rs` 校验端口、固定输入映射、邻接目标、出口及可达性；手写和动态生成计划共用该入口。运行版本不能修改。
- `graph/advance.rs`、`routing.rs`、`causal.rs` 以纯函数推进激活分支、局部路由与因果汇合。每次回流生成独立 visit，输入引用固定到具体 output 轮次；未选择分支不参与等待。
- `graph/outputs.rs` 保存实际内容、产物引用及独立的完成／验证依据。未知判断不能满足相应出口要求；根完成还要求全部激活分支收敛。
- `activation.rs` 分开全图生成和局部路由模型请求。路由只序列化连接输出、路由语义、相邻输入描述及出口要求，不发送根目标或完整计划。
- `execution/` 管理激活版本、控制 epoch、幂等回执、暂停取消与资源请求。先构造 Prepared 回执，再由节点持久化、派发；后继请求含映射内容和同轮次 `source_outputs`。
- 非法路由、缺少输出、访问上限及能力契约错误进入 blocked，不创建回退能力。每实例默认最多访问 20 次。
- 历史决策树和 Playbook 类型保留供只读查询，旧写入／调度入口统一报迁移错误；没有第二套执行内核。

## v3 事件驱动调度

- `crates/core/src/brain/scheduler.rs` 定义 schema v3 的最小运行、操作和事件投影。根请求仍以命名工程输入保存于根执行；脑状态不复制子执行输入、消息、DAG 或产物正文。
- `crates/brain/src/scheduler/` 以纯函数执行能力预筛、严格 `Dispatch`/`Complete`/`Fail` 校验、轮次屏障和终态处理。输入绑定只能引用根输入、成功执行的 `execution_id` 输出路径或已有产物。
- `crates/control/src/api/brain_runs/v3/` 每轮查询目录并判断一次，通过 `ExecutionGateway` 创建真实 Agent、Team、DAG、TODO 或 Operator；大脑停止轮询，只有节点确认的终态事件唤醒下一轮。
- Store 的 scheduler run/operation/event 三类记录支持 generation 栅栏、终态事件幂等和按序分页；事件只包含索引、引用和摘要。子执行详情通过 `GET /api/executions/{execution_id}` 查询。
- `crates/worker/src/brain/v3/` 负责恢复未确认事件、创建请求和取消请求。失败终态立即取消同轮兄弟操作，迟到事件只记账，不改写已经终态的脑状态。
- v2 运行保留只读兼容和原有回归路径；新运行必须明确 `schema_version: 3`。

## 索引
- `crates/core/src/brain/` — v2 计划、图状态、输出与路由 DTO，以及 v3 调度契约
- `crates/brain/tests/` — action_flow/execution/planning/ontology 与 scheduler_v3 行为回归

## 相关
- [features/brain](../../features/brain/index.md) — 工作台操作面
- [agents/control](../control/index.md) — 派发 API
- [agents/worker](../worker/index.md) — 持久化、能力输出适配与恢复
- [运行协议](../../docs/brain-orchestration.md) — 输入文档、输出、路由和迁移契约
