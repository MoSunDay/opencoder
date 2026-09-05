Commit: (working-tree, daemon 统一入口 + 全量签名 + SPA 内嵌)

# node 模块

## 职责
把一台机器变成集群的执行节点：`opencoder-agent --remote <server>` 常驻进程（原 `opencoder daemon --client`），用共享 token（`--token` 或 `OPENCODER_SERVER_TOKEN`，永不自动生成）对 server 全量 HMAC 签名注册，领取任务后**在本机配置与 LLM 凭证下**跑完整 agent session，事件实时回传 server。本地 libsql 同步落一份完整 transcript（经与 web drain 相同的 `spawn_event_flusher`，零额外代码）。

## 边界与非目标
- 只做出站 HTTP（heartbeat/claim/upload/status），**永不接受入站连接**；不信任 server 下发的任何模型/密钥——执行端凭证全在本地。
- v1 单节点同一时刻至多一个活跃任务（server 侧 claim 守卫），并行化是后续工作。
- 不做任务自动重派：节点失联的任务由 server 标 error 收束，人工重发。

## 关键抽象
- `uplink.rs`：`Uplink{http,base,token}` REST 客户端，所有请求经单一 `signed_request` 出口按共享 token 做 HMAC-SHA256 签名（`x-sig-timestamp`/`x-sig`）；请求形状即 `opencoder_core::node_protocol` DTO（register/heartbeat/claim/events/status 五口子；心跳走独立 5s 短超时预算（`HEARTBEAT_TIMEOUT`，最坏静默间隙 ≈ 5s 超时 + 5s tick < server `STALE_AFTER_MS`=20s，约 2× 余量；控制面其余请求仍 120s），心跳携带的 control 任务经 `tokio::spawn` 脱离 tick 关键路径（`Inflight` mutex 下原子去重兜并发））。
- `batcher.rs`：纯函数攒批器——32 条或 300ms 先到触发 flush；`push/should_flush/take` 可独立单测。
- `executor.rs`：领到任务 → 本地 `LibsqlStore` + 本地 `Config` 构造 `ChatStream` → 复用 session crate 原语（`resume_and_replay` + `run()` + 事件回调攒批上传）；取消传导走 runner 提供的 watch channel 触发本地 turn cancel。
- `runner.rs`：主循环——注册（同名顶替旧行）→ 心跳 tick(5s) 与 idle claim 轮询(1.5s) 双 interval select；任务串行执行。`client_override` 仅测试注入。

## 主流程
注册成功后进入双 timer 循环：心跳维持在线并取回 `cancel_task_ids`（一拍内送达执行中任务）；claim 轮询领 FIFO 任务 → executor 全程上传事件批（失败有界退避仅告警）→ 终态上报 done/error/cancelled（cancel 亦由 server 端收束帧闭流）。

## 相关模块
- server 半区：[agents/web](../web/index.md)（REST+SSE 桥、NodeHub broadcast）；协议：[agents/core](../core/index.md) `node_protocol.rs`；持久化：[agents/store](../store/index.md)（nodes/node_tasks 表 + 合成 session task_type="node"）。

## DAG 扩展点

- `DagHook` trait（claim/execute）：`NodeOpts.dag: Option<Arc<dyn DagHook>>`，由 `opencoder-agent` 注入（node crate 不依赖 VM/runc 链）。
- runner idle 轮询：无 prompt task 时尝试 DAG claim，单活跃 run 串行执行；专属 heartbeater 把应答里的 `cancel_run_ids` 翻成该 run 的 cancel flag。
- uplink 增补：`dag_claim`（GET /api/nodes/dag/claim，204→None）、`dag_events`（POST 批量事件）、`dag_status`（POST 终态）。
