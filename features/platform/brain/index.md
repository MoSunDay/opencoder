Commit: cea9499682d959503567277918b3f8da3d8115ba

# 大脑调度与运行历史

分层里程碑计划的调度与运行历史工作台；能力与调度实现以 brain 域卡为准。

## 索引

- [brain 域卡](../../../agents/orchestration/brain/index.md) — 能力目录与分层调度的完整
  代码索引
- [crates/brain/src/layered/](../../../crates/brain/src/layered) — 图校验、
  分层、上下文、决策、终态与重试
- [crates/brain/src/runtime.rs](../../../crates/brain/src/runtime.rs) — 能力库
  持久化与向量检索
- [api/brain_runs/](../../../crates/control/src/api/brain_runs)
  — 计划准入、派发、事件与目录（plans/runs/plan_capabilities/catalog）
- [crates/worker/src/brain/](../../../crates/worker/src/brain) — 节点侧投影、
  恢复、模型调度与确认
- [crates/web/src/api_brain.rs](../../../crates/web/src/api_brain.rs) — 工作台
  API
- [调度规则](../../../rules/06-brain-scheduling-contract.md)、
  [运行协议](../../../crates/control/src/api/brain_runs)
- [历史数据清理](../../../scripts/maintenance/brain_cleanup)、
  [验收说明](../../../scripts/acceptance/brain/README.md)

## 相关

- [control 域卡](../../../agents/platform/control/index.md)、
  [web 域卡](../../../agents/frontend/web/index.md)
- [调度平台](../agent-platform/index.md)
