Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# Agent Harness

opencode/codex 双执行器契约与资源作用域。细节以代码为准。

Operator 会话也可在创建时显式选择 Codex，并将启动 env 传给 Codex 进程及续会话；托管设置提供默认值，Operator 输入 env 可覆盖同名值。Operator 的隔离 HOME 最终生效；需要独立 Codex 登录目录时可注入 `CODEX_HOME`。

本地 TUI 可用 `opencoder tui --wrap codex --envs KEY=VALUE` 启动 Codex 会话；`--envs` 可重复，按原值传递。`/task` 新建任务沿用本次 TUI 的执行器与注入变量；恢复已有任务沿用其保存的运行态，显式重传的变量值必须与保存值一致。Codex 登录目录可通过 `CODEX_HOME` 指定。

[Windows TUI 与 Operator](../windows/index.md) 支持原生 `codex.exe`，沿用保存的线程与会话；取消、超时和节点退出清理该次执行的整个进程树。

[TUI 的 Server 任务](../agent-platform/index.md#tui-任务入口) 使用注册能力的执行器和托管设置；Operator 的 Codex 模型、环境与续会话由 Server 管理。恢复远端任务时，本地 `--wrap` 选择不会改变该绑定。

DAG 的静态 Agent 步和动态 Agent 实例均在本次共享 runc 容器内运行，并支持 Codex；默认复用实际执行节点的 Codex 登录态，显式 Harness/profile 可覆盖设置。Server 与节点分离时不自动分发 Server 登录文件。步骤模型优先于 profile 模型。

runc 需要预置原生 Codex CLI 及运行依赖，直接挂载节点登录目录以支持认证刷新；缺失依赖、认证失败或异常事件流使步骤失败。单步取消和超时只结束该步骤进程树，整次 DAG 结束才回收共享容器。纯 Codex DAG 不要求 OpenCoder 原生模型凭证。详见 [配置与 rootfs 制备](../../docs/registered-runners.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)。

## 相关
- [agents/core](../../agents/core/index.md) — Harness 类型与 Codex 设置
- [agents/session](../../agents/session/index.md) — 运行契约消费方
- [agents/local](../../agents/local/index.md)、[agents/tui](../../agents/tui/index.md)、[agents/web](../../agents/web/index.md)、[agents/worker](../../agents/worker/index.md)
