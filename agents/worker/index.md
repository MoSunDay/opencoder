Commit: d146e517f8b31ba3f8e5a1e493e0450d0c50d624

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器；`agent_how.rs` 是
  `kind=agent` 会话的 how.md 契约：`declared_how_append`（8 KiB 预算，创建时经
  harness envs 注入 `OPENCODER_HOW_APPEND`，与 DAG agent 步同机制）、成功终态
  `append_to_how_md`（warn-only 不改结果）、`transcript_tail`/`agent_result` 产出
  `output_text`/`output_json`（Operator/Maintenance 仍只回 `{"session_id"}`）
- `src/operations/maintenance.rs` — 维护命令执行：`dialogs_clear` 活跃执行跳过，
  其余 `delete_sessions` + `journal.forget`（防下一次全量 IndexReport 复活）
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
