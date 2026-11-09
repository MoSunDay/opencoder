Commit: 6c3491fd94ec48f28aa51058feb4dd7c9e3daf83

# dag-runtime 模块

节点侧 DAG 调度与执行。一次运行归属一个节点、一个 `runc` 容器；
Server 不链接此运行时。

## 索引
- [runtime.rs](../../../crates/dag-runtime/src/runtime.rs) — 单次认领运行的整体调度循环
- [runtime/scheduler.rs](../../../crates/dag-runtime/src/runtime/scheduler.rs) — 共享运行并发额度的调度器，轮转逻辑节点
- [runtime/input.rs](../../../crates/dag-runtime/src/runtime/input.rs) — 启动容器前冻结完整运行输入
- [runtime/dynamic.rs](../../../crates/dag-runtime/src/runtime/dynamic.rs) — 动态组展开与恢复
- [runtime/managed/](../../../crates/dag-runtime/src/runtime/managed/mod.rs) — 未终态操作保留不计入依赖终态表
- [control/](../../../crates/dag-runtime/src/control/mod.rs) — 节点本地命令邮箱；仅调度器写步骤状态
- [resources.rs](../../../crates/dag-runtime/src/resources.rs) — 运行资源固定（`frozen_resources`）
- [layout.rs](../../../crates/dag-runtime/src/layout.rs) — 单行 re-export
  （`pub use opencoder_dag::layout::run_parent`），实现锚点在
  [dag 模块](../dag/index.md)
- [sandbox/run/](../../../crates/dag-runtime/src/sandbox/run/mod.rs) — 容器装配：镜像校验、挂载（`mounts.rs`、`config.rs`）
- [sandbox/supervisor.rs](../../../crates/dag-runtime/src/sandbox/supervisor.rs) — 步骤进程树监督
- [sandbox/runc/](../../../crates/dag-runtime/src/sandbox/runc/cleanup.rs) — 容器删除回收与 init 退出核对
- [sandbox/codex/](../../../crates/dag-runtime/src/sandbox/codex/mod.rs) — Codex 启动契约：`broker.rs`、`startup.rs`、bundle
- [exec/](../../../crates/dag-runtime/src/exec/mod.rs) — 步骤执行器与共享管线（`StepCtx`、native、agent_runc 等）
- [step_io.rs](../../../crates/dag-runtime/src/step_io.rs) — 从运行循环提取的单步产物落盘
- [step_log.rs](../../../crates/dag-runtime/src/step_log.rs)、[dag_events.rs](../../../crates/dag-runtime/src/dag_events.rs) — 步骤输出落库与运行事件
- [exec/runc_events.rs](../../../crates/dag-runtime/src/exec/runc_events.rs) — 容器 Agent 事件导入子会话
- [exec/native/artifacts.rs](../../../crates/dag-runtime/src/exec/native/artifacts.rs) — 声明驱动的任务产物归档

## 镜像与工具
- [prepare-dag-rootfs.sh](../../../scripts/prepare-dag-rootfs.sh) — 镜像制备；配套 `scripts/dag-rootfs/` 安装脚本
- [examples/codex-startup-probe.rs](../../../crates/dag-runtime/examples/codex-startup-probe.rs) — 真实 runc 下的 Codex 启动探测
- [examples/connection-mount-probe.rs](../../../crates/dag-runtime/examples/connection-mount-probe.rs) — 连接只读挂载验证
- [tests/](../../../crates/dag-runtime/tests/sandbox_environment.rs) — 沙箱、preflight 与 run_loop 集成测试
- [runc_scheduling/main.py](../../../scripts/acceptance/runc_scheduling/main.py) — 两节点验收

## 相关
- [sandbox/codex](../../../crates/dag-runtime/src/sandbox/codex/mod.rs) 将固定的 Codex 启动脚本参数传入原生步骤；执行仍使用工作流容器和节点上的入口。
- [执行约定](../../../rules/04-dag-execution-contract.md)、[DAG 能力](../../../features/platform/dag/index.md)
- [dag-binary](../dag-binary/index.md)、[worker](../../execution/worker/index.md)
- [动态步骤](../../../crates/dag/src/dynamic.rs)、[Codex 启动](../../../crates/dag-runtime/src/sandbox/codex/mod.rs)、
  [rootfs 制备](../../../scripts/prepare-dag-rootfs.sh)
