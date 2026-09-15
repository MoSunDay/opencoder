Commit: 8a50a393cbe615f5d6453ff4290da0bf03546881

# dag-runtime 模块

节点侧 DAG 调度与执行：agent/wasm 两类步骤。

## 关键路径
- `src/runtime.rs` — 并发至多 4 步（`MAX_CONCURRENT_STEPS`），取消令牌传播；调度前 `ensure_run_session` 兜底建 run 会话行（`session_events.session_id` 是外键，缺行会静默丢掉整轮 `step_output`）。
- `src/checkpoint.rs` — 恢复仅由显式 resume 触发；done 且产物可读不重跑。
- `src/step_io.rs` — 产物/事件统一记录，写失败即步骤失败；`meta.json` 带可选 `session_id`，`write_session_artifact` 在会话创建瞬间落 `<step>/session.json`（只 warn，不失败步骤）。
- `src/step_log.rs` — 步骤输出镜像到 **Node Store** `session_events`：`sse_kind="step_output"`、payload `{step,stream,text,at_ms}`，300ms/4KiB 批量、单条 `text` ≤8KiB（按字符边界切）、终态前强制尾冲刷、写失败只 warn 丢弃。wasm 走 `runtime.rs` 注入（in-process sink 镜像 + runc 管道 tee），agent 步骤的事件走 session runner。
- `src/dag_events.rs` — 批量上报 8 条/300ms，终态前等待冲刷；agent transcript 的 `text_delta` 与步骤输出通过 `step_log` 增量上报（Uplink 实时链路，与 `step_log.rs` 的落库链路互补）。
- `src/exec/wasm/` — wasmtime WASI 模块；runc fail-closed，绝不回落 in_process。
- `src/exec/how_append.rs` — `OPENCODER_HOW_APPEND` 追加 agent 共享池 `how.md`。
- `src/sandbox/` — OCI bundle/rootfs 生成与有界清理。
- 事件经 `Uplink::for_local_dag` 写 Node Store（crates/node）；测试 `tests/run_loop.rs`。

## 边界
- `opencoder-server` 不链接；执行只发生在 claiming 节点。
- RustPython 已退役，由 wasm 步骤取代；`_modules` 为共享模块库保留目录名。

## 相关
- [agents/worker](../worker/index.md) — 本地 DAG 接入方。
- [agents/dag](../dag/index.md) — 契约来源。
