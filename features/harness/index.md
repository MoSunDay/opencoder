Commit: efe243b640c6e550770a3373e48c80773297bdfb

# Agent Harness

opencode/codex 双执行器契约与资源作用域。细节以代码为准。

DAG 的静态 Agent 步和动态 Agent 实例均支持 Codex，host 与 runc 都默认复用实际执行节点的 Codex 登录态；显式 Harness/profile 可覆盖设置。Server 与节点分离时不自动分发 Server 登录文件。步骤模型优先于 profile 模型。

runc 需要预置原生 Codex CLI 及运行依赖，直接挂载节点登录目录以支持认证刷新；缺失依赖、认证失败或异常事件流使步骤失败，取消和超时回收容器。纯 Codex DAG 不要求 OpenCoder 原生模型凭证。详见 [配置与 rootfs 制备](../../docs/registered-runners.md)。

## 相关
- [agents/core](../../agents/core/index.md) — Harness 类型与 Codex 设置
- [agents/session](../../agents/session/index.md) — 运行契约消费方
- [agents/local](../../agents/local/index.md)、[agents/web](../../agents/web/index.md)、[agents/worker](../../agents/worker/index.md)
