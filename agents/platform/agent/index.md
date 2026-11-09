Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口（fleet worker 二进制，main.rs）。

Windows 的 `opencoder-agent --remote` 提供原生 Operator 节点；安装与运行见
[Windows](../../../features/platform/windows/index.md)。

## 索引
- [main.rs](../../../crates/agent/src/main.rs) — 二进制入口；binarytime/runc 依赖链只在此处链接
- [host/mod.rs](../../../crates/agent/src/host/mod.rs) — Host 装配与 fleet channel
- [host/service.rs](../../../crates/agent/src/host/service.rs) — 维护命令中继与索引上报（`index_changes`）
- [host/client.rs](../../../crates/agent/src/host/client.rs) — 按 Runtime 归属的 RPC 转发；只读结果复用 result_reader
- [host/lifecycle.rs](../../../crates/agent/src/host/lifecycle.rs)（含 `lifecycle/activity.rs`）— retired Runtime 回收
- [host/runtime.rs](../../../crates/agent/src/host/runtime.rs) — Runtime inventory 状态查询
- [host/inventory/](../../../crates/agent/src/host/inventory/mod.rs) — 各 Runtime 清单变化的增量获取
- [host/deployment.rs](../../../crates/agent/src/host/deployment.rs)、[host/admission.rs](../../../crates/agent/src/host/admission.rs) — 发布与准入边界
- [executor.rs](../../../crates/agent/src/executor.rs) — Runtime 工作线程上限，降低空闲堆驻留
- [storage.rs](../../../crates/agent/src/storage.rs) — 数据目录布局迁移
- [host/tests/](../../../crates/agent/src/host/tests/forwarding.rs) — 转发回归等内联测试

## 相关
- [worker](../../execution/worker/index.md) — 转发目标与 result_reader 生产者
- [dag-runtime](../../workflow/dag-runtime/index.md) — 节点 DAG 执行
- [Windows](../../../features/platform/windows/index.md) — Operator 节点安装与运行
