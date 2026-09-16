Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器
- `crates/worker/src/state.rs` — runtime.db 与节点 ID
- `crates/worker/src/layout.rs`、`src/journal/` — 执行布局与原子落盘
- `crates/worker/src/runtime/capacity.rs`、`src/resources.rs` — Runtime 归属与资源固定
- `crates/worker/src/brain/` — 图激活、版本发布、局部路由、持久化回执、输出适配及唤醒。

## Brain 主流程

接受 v2 根运行后，通过有限激活调用 [brain 纯函数内核](../brain/index.md)，保存路由读集、选择、因果输入和 Prepared 回执，再由 outbox 派发。重启重放保留动作 ID，重复和乱序通知不重启任务；暂停、取消与资源互斥仍由原执行边界管理。

Agent/Operator 最后回答、Team 最终总结以及 DAG/TODO 的固定 `output_pointer` 统一适配具名 output；不能从能力内部状态推断业务通过。输出事件包含轮次与完成／验证依据，下游承接实际内容及同轮产物引用。

节点在恢复写入前只读检查旧 Brain 根、子执行与回执。发现旧非终态运行即拒绝启动新协议，历史终态记录保持可读。发布工具在候选准备和切换前执行对应检查；不需要新表或环境变量。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [运行协议](../../docs/brain-orchestration.md)
