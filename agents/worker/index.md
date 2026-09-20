Commit: 2f6b202def6c5e75d1411143570784a1576b3407

# worker 模块

节点执行面：接受/恢复执行、资源快照、workload 适配。

## 索引
- `crates/worker/src/service.rs` — 根执行与会话清单
- `crates/worker/src/workloads/` — agent/team/dag/todos/project 适配器；`dag.rs`
  的 `apply_input`（纯函数，带单测）把派发 input 折进冻结 spec：`prompt` 追加
  各 Agent 步「执行要求」、`args`（非空字符串）追加到各 Wasm 步 command
  （空白切分成 argv；每次 run/resume 基于冻结定义重新 decode，幂等）；
  `agent_how.rs` 是
  `kind=agent` 会话的 how.md 契约：显式 `how_append` 缺失时以首条 `prompt` 作为 how
  追加内容；`declared_how_append`（8 KiB 预算，创建时经 harness envs 注入
  `OPENCODER_HOW_APPEND`，与 DAG agent 步同机制）、成功终态
  `append_to_how_md`（warn-only 不改结果）、`transcript_tail`/`agent_result` 产出
  `output_text`/`output_json`（Operator/Maintenance 仍只回 `{"session_id"}`）
- `crates/worker/src/workloads/agent_runc.rs`（+ `agent_runc/events.rs` 事件尾随
  解析）— `run_mode: agent` 会话运行时：目标 agent 卡钉 `Agent` 模式时
  `kind=agent` 每个回合是一轮 runc 容器（`/usr/bin/agent-session-runner`、
  `ArgvStyle::Direct`，bundle 在 `<workflow root>/bundles/agent-sessions/<id>`，
  run root 种子 messages.json 全量 + 截断 events.ndjson + prompt.txt），host 只
  tail `events.ndjson` 逐行经 `SessionEvent::from_sse` 还原入库（SSE 中继可看），
  终局把 messages 增量折回 store；builtin agent 与缺卡/坏卡一律走 host。准入
  fail-closed（`create.rs` prepare 与每轮 `preflight` 双检：runc、`<workflow
  root>/rootfs`、LLM key）；`operations/command.rs` 与 `queue` 重放拦截 POST
  prompt（staged 进执行 input 由 launch 执行，回 202 accepted；steer/queue/
  compact/handoff 409），GET 保持原生。`events.rs`：只消费完整行、部分行留给
  下次、坏行跳过仍计 offset、sidecar 帧丢弃、error 帧取最后一条
- `src/operations/maintenance.rs` — 维护命令执行：`dialogs_clear` 接收控制面按
  Operator/Agent lane 筛选后的 id，活跃执行跳过，其余 `delete_sessions` +
  `journal.forget`（防下一次全量 IndexReport 复活）；两类执行都由同一
  Operator-capable 节点保存索引并以显式 `ExecutionRef` 提供明细路由
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

接受 schema v3 根运行后，worker 根节点维护调度 projection、generation、模型上下文和唤醒；control 只归一化能力目录、创建普通子执行并处理中继回执。子执行终态通过 outbox 发送给 control；节点重启只恢复未确认终态事件和未完成创建请求，失败终态取消同轮兄弟执行，迟到通知不改变已终态脑状态。

v3 Brain 的通用 execution events 查询从 scheduler event projection 读取事件索引，并以 `{seq, kind, data, ts}` 适配现有 SSE；事件只含能力、执行 ID、终态和摘要，子执行正文仍由对应 execution 查询维护。

Agent/Operator 最后回答、Team 最终总结以及 DAG/TODO 的固定 `output_pointer` 统一适配具名 output；不能从能力内部状态推断业务通过。输出事件包含轮次与完成／验证依据，下游承接实际内容及同轮产物引用。

节点在恢复写入前只读检查旧 Brain 根、子执行与回执。发现旧非终态运行即拒绝启动新协议，历史终态记录保持可读。发布工具在候选准备和切换前执行对应检查；不需要新表或环境变量。

## 相关
- [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)
- [运行协议](../../docs/brain-orchestration.md)
