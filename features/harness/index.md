Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# Agent Harness

opencode/codex 双执行器契约与资源作用域；类型与设置以 core 域卡为准。

## 索引

- [core 域卡](../../agents/core/index.md) — `Harness` 类型与 Codex 设置
  （[crates/core/src/harness/](../../crates/core/src/harness)，
  [settings.rs](../../crates/core/src/harness/settings.rs)）
- [session 域卡](../../agents/session/index.md) — 执行器准备与续会话校验
  （[crates/session/src/harness/](../../crates/session/src/harness)）
- [codex/launch.rs](../../crates/session/src/harness/codex/launch.rs)、
  [process.rs](../../crates/session/src/harness/codex/process.rs) — Codex
  进程启动与管理（`CODEX_HOME`）
- [codex/control/](../../crates/session/src/harness/codex/control) — 可选
  外部输入协议
- [workloads/agent.rs](../../crates/worker/src/workloads/agent.rs)
  — 节点侧 Agent/Operator workload
- [workloads/dag.rs](../../crates/worker/src/workloads/dag.rs)
  — DAG 内 Agent 步骤的共享容器执行
- [local 域卡](../../agents/local/index.md)、
  [tui 域卡](../../agents/tui/index.md)
  — 本地 TUI 的 `--wrap codex` 与 env 注入
- [web 域卡](../../agents/web/index.md)、
  [worker 域卡](../../agents/worker/index.md)
  — 远端 Operator 的托管设置与执行
- [Codex 启动脚本](../../docs/codex-startup-script.md)、
  [外部输入协议](../../docs/external-harness-control.md)
- [rootfs 制备](../../docs/registered-runners.md)、
  [DAG 执行约定](../../rules/04-dag-execution-contract.md)

## 相关

- [Windows TUI 与 Operator](../windows/index.md)、
  [调度平台](../agent-platform/index.md)、[原生 DAG](../dag/index.md)
