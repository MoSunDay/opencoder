Commit: f9e0147d717d65dc90df6d0b6d83e5360a81fcb7

# Agent Harness

opencode/codex 双执行器契约与资源作用域；类型与设置以 core 域卡为准。

## 索引

- [core 域卡](../../../agents/foundation/core/index.md) — `Harness` 类型与 Codex 设置
  （[crates/core/src/harness/](../../../crates/core/src/harness)，
  [settings.rs](../../../crates/core/src/harness/settings.rs)）
- [session 域卡](../../../agents/runtime/session/index.md) — 执行器准备与续会话校验
  （[crates/session/src/harness/](../../../crates/session/src/harness)）
- [codex/launch.rs](../../../crates/session/src/harness/codex/launch.rs)、
  [process.rs](../../../crates/session/src/harness/codex/process.rs) — Codex
  进程启动与管理（harness `envs` 通用透传，无 `CODEX_HOME` 特判）
- [sandbox/codex/](../../../crates/dag-runtime/src/sandbox/codex)、
  [core/src/harness/mod.rs](../../../crates/core/src/harness/mod.rs) —
  `CODEX_HOME` 字面量 env 处理（沙箱继承白名单与 harness env 匹配）
- [codex/control/](../../../crates/session/src/harness/codex/control) — 可选
  外部输入协议
- [workloads/agent.rs](../../../crates/worker/src/workloads/agent.rs)
  — 节点侧 Agent/Operator workload
- [workloads/dag.rs](../../../crates/worker/src/workloads/dag.rs)
  — DAG 内 Agent 步骤的共享容器执行
- [local 域卡](../../../agents/frontend/local/index.md)、
  [tui 域卡](../../../agents/frontend/tui/index.md)
  — 本地 TUI 的 `--wrap codex` 与 env 注入
- [web 域卡](../../../agents/frontend/web/index.md)、
  [worker 域卡](../../../agents/execution/worker/index.md)
  — 远端 Operator 的托管设置与执行
- [Codex 启动脚本](../../../docs/codex-startup-script.md)、
  [外部输入协议](../../../docs/external-harness-control.md)
- [rootfs 制备](../../../docs/registered-runners.md)、
  [DAG 执行约定](../../../rules/04-dag-execution-contract.md)

## 相关

- [Windows TUI 与 Operator](../../platform/windows/index.md)、
  [调度平台](../../platform/agent-platform/index.md)、[原生 DAG](../../platform/dag/index.md)
