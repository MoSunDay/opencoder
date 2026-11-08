Commit: cea9499682d959503567277918b3f8da3d8115ba

# Agent 调度平台

Server 控制面与节点执行面的调度总纲；本卡只做指路，细节以代码与域卡为准。

## 索引

- [control 域卡](../../../agents/platform/control/index.md) — 控制面：节点接入、全局
  定义、执行索引与调度
- [crates/control/src/admission.rs](../../../crates/control/src/admission.rs)、
  [capabilities.rs](../../../crates/control/src/api/executions/capabilities.rs)
  — 执行准入与节点能力校验
- [crates/control/src/scheduler.rs](../../../crates/control/src/scheduler.rs)、
  [scheduler/](../../../crates/control/src/scheduler) — 调度循环与日程扫描
- [resource_admission/](../../../crates/control/src/api/resource_admission)
  — `_resource_request` 资源授权准入
- [crates/control/src/release/](../../../crates/control/src/release) — 平滑发布
  信号、中继与资源切换
- [server 域卡](../../../agents/platform/server/index.md) — Server 二进制入口
- [node 域卡](../../../agents/platform/node/index.md) — 节点接入与执行转发
- [agent 域卡](../../../agents/platform/agent/index.md) — 节点执行程序
  `opencoder-agent`
- [worker 域卡](../../../agents/execution/worker/index.md) — 受理/恢复、资源快照与
  workload 适配；[health.rs](../../../crates/worker/src/runtime/health.rs)
  节点容量健康检查，[journal.md](../../../agents/execution/worker/journal.md) 执行日志
- [web 域卡](../../../agents/frontend/web/index.md) — Web 控制台 API
- [session 域卡](../../../agents/runtime/session/index.md) — Operator 规则与会话隔离
- [crates/tui/src/worker.rs](../../../crates/tui/src/worker.rs) — TUI 的
  Server 任务入口
- [平滑发布验收](../../../scripts/acceptance/smooth_release/main.py)
- [会话索引维护](../../../scripts/maintenance/session_index/README.md)、
  [UI 验收约定](../../../rules/05-ui-acceptance.md)

## 相关

- [原生 DAG](../dag/index.md)、[Windows 节点](../windows/index.md)
- [项目工作台](../project/index.md)、[Ontology](../ontology/index.md)
