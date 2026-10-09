Commit: 36c8cb854f21d4977ec3c336af901588c6fac133

# 外部资源授权与 Server 等待队列

Server 通过通用资源提供方约束执行派发。携带 `_resource_request` 的提交先固定身份、定义和输入，再由 outbox 按时间顺序尝试资源授权；未获授权不创建节点执行。普通任务保留原有节点容量预留与派发顺序，不因外部资源服务故障阻塞。复用既有 Fleet 存储，不新增表。

| 实现 | 测试 |
| --- | --- |
| `resource_admission::ready`、`cancel_waiting`、`command` | [HTTP 回归](../../../crates/control/tests/resource_admission.rs)：持久等待、取消不接触节点、缺失提供方保留提交 |
| `outbox::resources::dispatch`、`executions::dispatch_queued` | [资源 E2E](../../../crates/control/tests/e2e/resource_admission.rs)：按序派发、不重复创建节点执行、资源提供方不可用时普通提交仍派发 |
| 普通任务 `submit` 预留节点容量 | 同一 E2E 的 `ordinary_submission_dispatches_while_resource_authority_is_unavailable` |

验证：workspace build、全量测试（5,787 通过、0 失败、8 忽略）及 clippy `-D warnings` 通过；真实 runc 资源队列验收通过；相同候选单独完成平滑切换、回滚及 900 秒观察。先前共享负载下出现普通容器清理超时，未修改既有阈值；单次通过不能证明超时原因。候选来自该基线的未提交源码，未执行生产发布。

相关：[control](../../../agents/control/index.md)、[平台功能](../../agent-platform/index.md)、[发布门槛](../../../rules/02-regression-gate.md)。
