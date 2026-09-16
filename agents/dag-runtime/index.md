Commit: a750c6634f4d9cc447a59a63d59a2c8d20da5b4c

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接。

## 索引
- `src/runtime.rs` — 步骤调度（并发上限、取消传播）
- `src/exec/` — wasm（wasmtime WASI）与 agent 步执行；agent 步产出经 `extract_output_json_from` 三级提取：```json 围栏 → 尾部裸 JSON 兜底（`extract_tail_bare_json`，string-aware 括号平衡、取最后一个可解析顶层对象；坏围栏时从围栏体之后扫描）→ 整段解析
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与批量上报
- `src/sandbox/` — OCI bundle/rootfs

## 边界
- 执行只发生在 claiming 节点；runc fail-closed，不回落 in_process。
