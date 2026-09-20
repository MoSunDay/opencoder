Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接，执行只发生在 claiming 节点。

## 索引
- `src/runtime.rs`、`src/runtime/` — 调度、动态展开与恢复
- `src/exec/` — wasm 与 agent 步执行（含产出提取）
- `src/exec/agent_runc.rs`、`src/sandbox/` — runc 沙箱（fail-closed）与 rootfs/挂载装配
- `src/exec/how_copy.rs`、`src/exec/runc_events.rs` — 冻结资源副本与容器事件导入
- `src/exec/wasm/host_imports*` — `opencoder` host imports
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与事件上报
- `examples/agent-step-runner.rs`、`examples/agent-session-runner.rs` — 容器内 session runner
- `examples/wasmtime-cli.rs`、`scripts/prepare-dag-rootfs.sh` — WASI 运行器与 rootfs 制备

## 相关
- [动态步骤说明](../../docs/dag-dynamic.md) — 实例 API 与恢复契约
