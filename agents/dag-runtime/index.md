Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接。

## 索引
- `src/runtime.rs` — 步骤调度（并发上限、取消传播）
- `src/exec/` — wasm（wasmtime WASI）与 agent 步执行
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与批量上报
- `src/sandbox/` — OCI bundle/rootfs

## 边界
- 执行只发生在 claiming 节点；runc fail-closed，不回落 in_process。
