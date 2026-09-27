Commit: 2aa44247d199d782881b9ee64921c4c6de2e6199

# Operator 支持 Codex 与启动环境变量

Operator 会话创建时可在页面选择 Codex，并通过 `KEY=VALUE` 逐行注入环境变量。`POST /api/sessions` 的 `harness` 与 `envs` 传至节点准入和持久化的 Harness 运行态，续会话沿用。显式 env 覆盖托管 Codex 同名值，Operator 私有 HOME 保持最高优先级。

会话页在窄屏以纵向布局保留输入区；SPA 产物与源码一同更新，供 Rust 服务端编译时嵌入。

## 相关

- [Agent 调度平台](../../agent-platform/index.md)
- [Agent Harness](../../harness/index.md)
