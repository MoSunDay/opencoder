Commit: 586e56014aaa9d2ec59047a24a8cb5ca644d1249

# brain 模块

能力目录、不可变 v2 计划与事件驱动 v3 调度器；能力自身负责执行与验证。
契约：运行版本不可修改；非法路由/缺输出/访问上限/能力契约错误进入 blocked、不建回退能力；v2 只读兼容，新运行须 `schema_version: 3`。

## 索引
- `crates/core/src/brain/` — v2 计划/图/输出/路由 DTO、v3 调度契约（`scheduler.rs` SchedulerPlan）
- `crates/brain/src/graph/` — 校验/推进/路由/因果/输出（纯函数）
- `crates/brain/src/activation.rs`、`crates/brain/src/execution/` — 模型请求拆分与执行回执/暂停取消
- `crates/brain/src/ontology/`、`crates/brain/src/playbook/` — 旧本体与 Playbook 只读兼容
- `crates/brain/src/scheduler/` — 能力预筛与严格 Dispatch/Complete/Fail 校验
- `crates/control/src/api/brain_runs/v3/` — 能力目录归一与 ExecutionGateway 派发
- `crates/worker/src/brain/v3/` — 节点本地投影恢复与事件确认
- `crates/brain/tests/` — action_flow/execution/planning/ontology/scheduler_v3 回归

## 相关
- [features/brain](../../features/brain/index.md) — 工作台操作面
- [agents/control](../control/index.md) — 派发 API
- [agents/worker](../worker/index.md) — 持久化、能力输出适配与恢复
- [运行协议](../../docs/brain-orchestration.md) — 输入/输出/路由/迁移契约
