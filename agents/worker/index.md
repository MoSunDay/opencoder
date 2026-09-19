Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器；`agent_how.rs` 是
  `kind=agent` 会话的 how.md 契约：显式 `how_append` 缺失时以首条 `prompt` 作为 how
  追加内容；`declared_how_append`（8 KiB 预算，创建时经 harness envs 注入
  `OPENCODER_HOW_APPEND`，与 DAG agent 步同机制）、成功终态
  `append_to_how_md`（warn-only 不改结果）、`transcript_tail`/`agent_result` 产出
  `output_text`/`output_json`（Operator/Maintenance 仍只回 `{"session_id"}`）
- `src/operations/maintenance.rs` — 维护命令执行：`dialogs_clear` 活跃执行跳过，
  其余 `delete_sessions` + `journal.forget`（防下一次全量 IndexReport 复活）
- `crates/worker/src/state.rs` — runtime.db 与节点 ID
- `crates/worker/src/layout.rs`、`src/journal/` — 执行布局与原子落盘
- `crates/worker/src/runtime/capacity.rs`、`src/resources.rs` — Runtime 归属与资源固定
- `crates/worker/src/brain/` — 图激活、版本发布、局部路由、持久化回执、输出适配及唤醒。
- `crates/worker/src/brain/workdir.rs` — 生效工作空间接缝：非 brain 工作负载的会话
  cwd、配置发现（`configuration()`→`Config::load`）与会话归属（`workdir_hash`）都经
  `node_workdir()` 解析（`scheduling.json` 的 `workdir` 优先，须绝对路径、空白归一
  null，否则节点启动 workdir）；`_brain` 根执行固定用执行目录下 `workspace/`。
  保存时立即建目录，启动时确保存在，缺失仅告警降级不阻断节点启动；workdir 经
  `PUT /api/nodes/:id/scheduling`（maintenance `configure_scheduling`）设置。

## Brain 主流程

接受 v2 根运行后，通过有限激活调用 [brain 纯函数内核](../brain/index.md)，保存路由读集、选择、因果输入和 Prepared 回执，再由 outbox 派发。重启重放保留动作 ID，重复和乱序通知不重启任务；暂停、取消与资源互斥仍由原执行边界管理。

接受 schema v3 根运行后，worker 只维护根执行上的调度意图、唤醒/派发/取消回执，并把子执行终态通过 outbox 发送给 control。control 的 scheduler projection 保存轮次和 operation 索引，子执行正文留在对应节点；节点重启只恢复未确认终态事件和未完成创建请求，失败终态取消同轮兄弟执行，迟到通知不改变已终态脑状态。

Agent/Operator 最后回答、Team 最终总结以及 DAG/TODO 的固定 `output_pointer` 统一适配具名 output；不能从能力内部状态推断业务通过。输出事件包含轮次与完成／验证依据，下游承接实际内容及同轮产物引用。

节点在恢复写入前只读检查旧 Brain 根、子执行与回执。发现旧非终态运行即拒绝启动新协议，历史终态记录保持可读。发布工具在候选准备和切换前执行对应检查；不需要新表或环境变量。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [运行协议](../../docs/brain-orchestration.md)
