Commit: (working-tree, multi-node distributed execution)

# 多节点分布式执行（server ↔ node）

## 背景

此前 opencode 只有单机 web server：算力受限于 server 所在机器，浏览器也只能直播本机 session。本轮扩展出集群能力——任意机器运行 `opencode node --remote <server>` 即可作为执行节点注册进集群；web UI「Nodes」面板实时展示节点状态、下发 prompt，任务在**节点本地配置与 LLM 凭证**下执行，事件流实时回传浏览器直播全过程。节点注册表、任务记录、执行事件全量落 server 端 libsql：server 重启不丢、页面刷新可回放历史。

## 核心复用决策

每个远程任务在 server 端对应一个**合成 session 行**（`task_type="node"`），节点回传的事件以原始 kind 字符串经 `SessionEventRecord` 直接写入现有 `session_events` 表 ⇒ SSE 回放 / 两级去重 / Last-Event-ID 断线续传机制零改动复用。

## 实现

### Phase 1 — 存储层（crates/store，schema v11 → v12）

- 新表 `nodes`（name UNIQUE，同名重注册 ON CONFLICT 顶替旧行且**保留原 id**，旧任务外键不断链）与 `node_tasks`（status/cancel_requested/created_at/claimed_at/finished_at + `idx_node_tasks_node_status`）。
- `node_state.rs::transition_allowed` 纯函数状态机格：pending→running→done|error|cancelled；running/pending 经 cancelling 收束；终态冻结。
- Store trait 新增 11 个 node 方法（全部带 default impl，测试 fake 零破坏），实现在 `libsql_store/nodes.rs`：
  - `claim_next_node_task`：BEGIN IMMEDIATE 内 单活跃守卫 → 最老 pending → 条件 UPDATE CAS（禁 RETURNING）；FIFO 以 `(created_at, rowid)` 定序——同毫秒 ULID 无单调性不可作 tiebreak（实测修复过一个该缺陷）。
  - `heartbeat_node`：touch + 取回 `cancelling AND cancel_requested=1` 任务 id 清单（取消指令一拍送达）。
  - `dispatch_node_task`：单事务内建 pending 任务 + 合成 session（task_type="node"）。
- 节点失联任务标 error 不自动重派（可靠性优先，人工重发）。

### Phase 2 — 协议与服务端

- `core/src/node_protocol.rs`：8 个纯数据 DTO + serde 测试。
- web 新模块：`nodes_state.rs`（NodeHub：task_session_id→broadcast 映射；staleness 20s 按 **server 收包时钟**记账）、`api_nodes.rs`（注册表半区）、`api_nodes_ops.rs`（claim/事件上传/终态上报）、`sse_nodes.rs`（浏览器 SSE 桥，先订阅后查库 + 强制复用 `sse_dedup::forward_live`，禁止复制去重逻辑）。
- 终态上报追加 done/error 收束帧闭流；取消语义：UI cancel → `cancel_requested` 落库 → 心跳应答带回（≤1 心跳周期）→ 本地 turn-cancel，事件照常收尾落库。
- 守卫：合成 session 被 prompt/agent/model/interrupt/fork/compact/handoff/skill 等 mutation 端点一律 409；`list_sessions` 即使 include_subagents 也排除 node 型。

### Phase 3 — 节点运行时（新 crate crates/node）

- `uplink.rs`（对齐 client/remote.rs 的代理绕行模式）/ `batcher.rs`（32 条或 300ms 攒批，纯函数可单测）/ `executor.rs`（resume_and_replay + run() + 事件回调攒批上传，取消 watch channel 触发本地 turn cancel）/ `runner.rs`（注册→心跳 5s + claim 轮询 1.5s 双 interval select，任务串行，SIGTERM 优雅退出）。
- CLI：`opencode node --name --remote [--token|--env OPENCODER_SERVER_TOKEN]`（token 语义同 client：绝不自动生成）。
- 事件映射直接调用 session crate 的 `SessionEvent::sse_kind()/sse_data()` 公共访问器 ⇒ 与主会话 SSE 字节级一致，零提炼成本。

### Phase 4 — UI 与验证

- 前端 `assets/nodes_panel.js`（383 行，未动 chat.js）：Nodes tab 3s 轮询 + 状态圆点（lost 红/busy 橙/idle 绿）+ 下发表单（model 下拉吃现成 /api/models）+ 直播视图（TextDelta 正文聚合、ToolStart/End 时间线、cancel 按钮 running 可见性切换）+ 历史条目 after=0 全量回放。
- 两层自动化：DOM shim 场景（shim 提炼为 dom_shim.mjs 复用，frontend_smoke 行为零变化）+ 进程级 e2e（真 build_app server × 注入 MockChat 的真 node runner：注册→派发→领取→流式回传→断流按 after= 续传无丢失→取消传导→终态→回 idle，SSE 帧序与 store 对账）。
- 冒烟脚本 `scripts/smoke_nodes.sh`：双进程起 server+node，3 个 ✅ 检查点（注册/dispatch 回执/终态），供人工复核。

## 边界

v1 每节点串行单活跃任务（busy 时其余留 FIFO 队列）；并行化、crates/client 保持瘦契约不动（常驻逻辑独立成 crate）均为有意取舍。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 节点 CRUD/重注册复用 id | `register_list_get_delete_roundtrip` / `duplicate_register_reuses_node_id` | `store/tests/nodes_store.rs` / `web/tests/nodes_api.rs` |
| 同名顶替保留外键 | `delete_node_cascades_to_tasks_and_synthetic_sessions` | `store/tests/nodes_store.rs` |
| 合成 session task_type=node 且对外隐藏 | `dispatch_creates_synthetic_session_with_task_type_node` / `dispatch_creates_task_and_hidden_synthetic_session` | store / web |
| claim FIFO+单活跃+跨节点隔离 | `claim_is_fifo_single_active_and_per_node_isolated` | `store/tests/nodes_store.rs` |
| 并发 claim 不重复发放 | `concurrent_claims_never_double_dispatch` | `store/tests/nodes_store.rs` |
| 状态机非法迁移拒绝 | `status_transition_grid_rejects_illegal_moves` | `store/tests/nodes_store.rs` |
| 取消幂等+心跳投递 | `request_cancel_is_idempotent_and_heartbeat_delivers_it` / `cancelling_pending_task_completes_immediately_and_streams_done` / `cancelling_running_task_answers_202_then_travels_via_heartbeat` | store/web |
| 事件回放游标与 seq 对账 | `uploaded_events_replay_with_monotonic_seqs_and_cursor` | `web/tests/nodes_ops.rs` |
| e2e 全状态机闭环 | `dispatch_stream_cancel_and_node_state_machine` | `web/tests/nodes_e2e_flow.rs` |
| 断流续传零丢失零重复 | `mid_stream_disconnect_resumes_without_loss_or_duplication` | `web/tests/nodes_e2e_reconnect.rs` |
| DTO serde 往返 | `serde_*` 系列 | `core/src/node_protocol.rs` 内联 |
| 攒批边界（32 条/300ms/take 清窗） | batcher 内联 4 用例 | `node/src/batcher.rs` |
| 节点 runner happy path | `claims_executes_uploads_and_reports_done` | `node/tests/runner_happy.rs` |
| 取消传导至本地终止 | `heartbeat_cancellation_reports_cancelled` | `node/tests/runner_cancel.rs` |
| UI DOM shim 场景 | render/payload/live/cancel/replay 断言 | `web/tests/frontend_nodes.mjs` |

- 全量回归：`cargo test --workspace` → **3346 passed / 0 failed**（231 suites）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 前端 harness：`frontend_smoke.mjs` 与 `frontend_nodes.mjs` 均 exit 0

## 失联节点任务收束（Risk#1 补账，error(node lost) 承诺落地）

### 动机
节点崩溃后其 `running`/`cancelling` 任务成为僵尸行：claim 单活跃守卫（状态集含
`running`/`cancelling`）会被永久阻塞，worker 以同名重注册也绕不开；此前唯一出口是
无人知晓的运维手工 `POST /api/nodes/tasks/:tid/status`。计划 Risk#1 明文承诺
「分区双执行：节点失联任务标记 error(node lost)」，本次补齐该验收行为。

### 落点与语义
- **store**：新增 `Store::converge_lost_node_tasks(now_ms, stale_ms)`（实现
  `libsql_store/nodes.rs::converge_lost`）。单事务（BEGIN IMMEDIATE）内：
  JOIN 选取「所属节点心跳距今 **严格大于** stale_ms（与展示层 `compute_status`
  排他边界一致）且任务状态 ∈ running/cancelling」的僵尸行 → 条件 UPDATE 置
  `error("node lost")` + 盖 `finished_at`（无 RETURNING，同 claim 约定）→ 释放
  忙位（`last_status='idle'`）→ 回读完整记录返回。终态天然冻结、幂等（二次调用
  返回空）；`pending` 与新鲜节点不受影响。
- **web**：`GET /api/nodes` 读路径即维护（opportunistic sweep，无后台 sweeper）——
  先调上述收束，再对每条收束记录复用 `emit_closure` 持久化并向 NodeHub 广播
  `sse_kind="error"` 终帧，直播页据此闭流；随后列表才组装，响应即时反映 idle 化。
  迟到的真 worker 上传会被终态冻结拒绝——分区双执行下以 server 标记为准。

### 测试清单（当次实跑）

| 行为 | 用例 | 文件 |
|---|---|---|
| 超时节点 running+cancelling 双收束+忙位释放 | `stale_node_running_and_cancelling_both_collapse` | `store/tests/nodes_lost_converge.rs` |
| 新鲜节点（恰差 1ms）不动 | `fresh_node_running_is_untouched` | 同上 |
| 终态冻结 + 二次收束为空（幂等） | `terminal_rows_frozen_and_second_sweep_is_empty` | 同上 |
| 排他边界（==stale 不收束）+ pending 存活 | `boundary_equality_and_pending_survive` | 同上 |
| 读路径收束：error("node lost")+尾帧+队列解卡 | `sweep_converges_running_task_and_unblocks_queue` | `web/tests/nodes_lost_sweep.rs` |
| 直播 SSE 收到 error 终帧闭流 | `live_sse_view_receives_error_closure_during_sweep` | 同上 |
| 两进程冒烟脚本接入 cargo | `smoke_script_two_process_nodes_flow_passes` | `tests/nodes_smoke_proc.rs` |

- 全量回归：`cargo test --workspace --no-fail-fast` → **3353 passed / 0 failed**
  （234 suites）
- 门禁：`cargo fmt --all --check` 通过；`cargo clippy --workspace --all-targets
  -- -D warnings` 零警告
- 冒烟脚本注入点：`OPENCODER_SMOKE_BIN` / `OPENCODER_SMOKE_PORT`（不设则行为
  与原 release 构建路径完全一致），默认仍可手动 `bash scripts/smoke_nodes.sh`
