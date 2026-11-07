Commit: 5afb497be53de0f763f06c26a1eb57709198874d

# DAG 工作流

画布/JSON/API 提交依赖图；原生二进制与 Agent 步骤在共享 runc 容器执行，
本卡只做指路。

## 索引

- [dag 域卡](../../../agents/workflow/dag/index.md) — 定义、域与协议
- [dag-runtime 域卡](../../../agents/workflow/dag-runtime/index.md) — 运行器、沙箱与
  恢复
- [dag-binary 域卡](../../../agents/workflow/dag-binary/index.md) — 二进制不可变版本池
- [crates/dag/src/spec.rs](../../../crates/dag/src/spec.rs) — 定义与
  `max_concurrency`
- [crates/dag/src/dynamic.rs](../../../crates/dag/src/dynamic.rs) — 动态节点
  展开
- [crates/dag-runtime/src/sandbox/](../../../crates/dag-runtime/src/sandbox) —
  runc 沙箱与工作区布局
- [workloads/dag.rs](../../../crates/worker/src/workloads/dag.rs)
  — 节点侧 DAG workload
- [capabilities.rs](../../../crates/control/src/api/executions/capabilities.rs)
  — 节点能力声明校验（`dag_container_v1`、`dag_dynamic_v1`）；
  `execution_io_v1` 常量位于
  [core/src/brain/capability.rs](../../../crates/core/src/brain/capability.rs)
  （`EXECUTION_IO_CAPABILITY`）
- [crates/web/src/api_dag.rs](../../../crates/web/src/api_dag.rs)、
  [api_dag_binaries.rs](../../../crates/web/src/api_dag_binaries.rs) — 画布/
  JSON 编辑与目录 API
- [执行规则](../../../rules/04-dag-execution-contract.md)、
  [动态步骤](../../../crates/dag/src/dynamic.rs)、
  [步骤控制](../../../crates/dag/src/control/mod.rs)
- [镜像与 rootfs 制备](../../../scripts/prepare-dag-rootfs.sh)、
  [目录去重与清理](../../../scripts/maintenance/catalog_cleanup/README.md)

## 相关

- [调度平台](../agent-platform/index.md)、[Harness](../../interaction/harness/index.md)、
  [平滑发布验收](../../../scripts/acceptance/smooth_release/main.py)
