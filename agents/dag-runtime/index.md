Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# dag-runtime 模块

节点侧 DAG 调度与执行：agent/wasm/runner 三类步骤。

## 关键路径
- `src/runtime.rs` — 并发至多 4 步（`MAX_CONCURRENT_STEPS`），取消令牌传播。
- `src/checkpoint.rs` — 恢复仅由显式 resume 触发；done 且产物可读不重跑。
- `src/step_io.rs` — 产物/事件统一记录，写失败即步骤失败。
- `src/dag_events.rs` — 批量上报 8 条/300ms，终态前等待冲刷。
- `src/exec/wasm/` — wasmtime WASI 模块；runc fail-closed，绝不回落 in_process。
- `src/exec/how_append.rs` — `OPENCODER_HOW_APPEND` 追加 agent 共享池 `how.md`。
- `src/exec/runner/` — 注册前台进程，NDJSON 事件流 + 可重放完成收据。
- `src/sandbox/` — OCI bundle/rootfs 生成与有界清理。
- 事件经 `Uplink::for_local_dag` 写 Node Store（crates/node）；测试 `tests/{run_loop,runner}.rs`。

## 边界
- `opencoder-server` 不链接；执行只发生在 claiming 节点。
- RustPython 已退役，由 wasm 步骤取代；`_modules` 为共享模块库保留目录名。

## 相关
- [agents/worker](../worker/index.md) — 本地 DAG 接入方。
- [agents/dag](../dag/index.md) — 契约来源。
- [注册 Runner](../../docs/registered-runners.md) — 协议与限制。
