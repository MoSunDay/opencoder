Commit: e50ffc433bca866fd17bd571a74f1bdf17705dea

# worker 模块

节点执行面：受理、排队、恢复与工作负载适配。

## 关键路径

- `crates/worker/src/operations/create.rs` — admission 锁内去重、预检并落盘
- `crates/worker/src/operations/queue/` — 单调序号 + FIFO/LIFO 选等待任务
- `crates/worker/src/operations/launch.rs` — 取容量后启动
- `crates/worker/src/operations/query/` — 明细/事件/消息分页；head_seq 水位
- `crates/worker/src/operations/query/dag_steps.rs` — DAG run 步级 meta.json 进度/单步视图
- `crates/worker/src/operations/query/project/` — prun-* 回放；载荷 64 KiB 分块
- `crates/worker/src/operations/query/runner.rs` — Runner 阶段/verdict/投递状态
- `crates/worker/src/operations/project_admission/` — Plan/Execute 独立 run ID
- `crates/worker/src/operations/maintenance.rs` — 维护工具；configure_scheduling
- `crates/worker/src/brain/` — TODO 存储根状态/事件，短 runc 激活、持久 outbox、同盘恢复；输出归一化与可下载产物。
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器；operator 复用 agent 循环（宿主机进程直跑，无 runc/无 node_maintenance）
- `crates/worker/src/runtime/scheduling.rs` — scheduling.json 持久化并发/队列序
- `crates/worker/src/state.rs` — runtime.db；节点 ID 持久化、目录锁
- `crates/worker/src/layout.rs` — `<kind>/<id>/execution.json` 布局
- `crates/worker/src/journal/` — 原子落盘（sync_all + rename）
- `crates/worker/tests/harness_matrix.rs` — 五类 Harness 预检与取消矩阵
- `crates/worker/tests/project_replay.rs`、`runner_dispatch.rs` — 端到端契约
- `scripts/acceptance/business/`、`project/` — 真实 NFS 与业务验收

## 边界

- 运行不依赖 WebSocket 存活；普通执行重启后的 interrupted 需显式 resume。Brain 根按持久唤醒/动作恢复，保持暂停与取消意图；不迁移丢盘节点所有权。
- Brain 激活等待时释放容量；状态与因果事件原子提交，通知游标去重。取消等待在途子执行回执，不能只结束根循环就宣称取消完成。
- `layout::ALL_KINDS` 必须覆盖全部有 kind 根目录的执行类型（含 operator）——漏一个即重启丢记录。
- Node 不开放入站 HTTP：agent 复用 web session API 进程内调用。
- system 团队执行已退役，create 直接拒绝。

## 相关

- [brain](../brain/index.md) 本体状态机；[工作台](../../features/brain/index.md) 产品边界
- [node](../node/index.md) NodeService 通道；[session](../session/index.md) 会话引擎
- [todos](../todos/index.md)、[dag-runtime](../dag-runtime/index.md)、[project](../project/index.md)
- [Agent 平台](../../features/agent-platform/index.md)、[Agent Harness](../../features/harness/index.md)
