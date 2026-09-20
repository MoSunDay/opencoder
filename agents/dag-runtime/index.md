Commit: efe243b640c6e550770a3373e48c80773297bdfb

# dag-runtime 模块

节点侧 DAG 调度执行；server 不链接，执行只发生在 claiming 节点。

## 索引
- `src/runtime.rs`、`src/runtime/` — 调度、动态展开与恢复
- `src/exec/` — wasm 与 agent 步执行（含产出提取）
- `src/exec/wasm/in_process.rs` — 先设置 Store 的 epoch 截止点，再启动时钟线程，避免初始化阶段丢失取消；执行前已取消的令牌直接返回 Cancelled，不进入 guest。
- `src/exec/agent_runc.rs`、`src/sandbox/` — runc 沙箱（fail-closed）与 rootfs/挂载装配
- `src/sandbox/codex/` — 解析节点 Codex 登录目录与冻结 Harness/profile，校验 guest 可执行文件；原登录目录直接读写挂载，私有启动配置独立于 DAG 产物。`agent_runc` 保存线程回执、导入事件，并使用容器内知识库路径；纯 Codex 不创建原生模型请求。
- `src/exec/how_copy.rs`、`src/exec/runc_events.rs` — 冻结资源副本与容器事件导入
- `src/exec/wasm/host_imports*` — `opencoder` host imports
- `src/step_log.rs`、`src/dag_events.rs` — 输出落库与事件上报
- `examples/agent-step-runner.rs`、`examples/agent-session-runner.rs` — 容器内 session runner
- `examples/wasmtime-cli.rs`、`scripts/prepare-dag-rootfs.sh` — WASI 运行器与 rootfs 制备

## 相关
- [动态步骤说明](../../docs/dag-dynamic.md) — 实例 API 与恢复契约
- [Codex DAG 接入](../../docs/registered-runners.md) — host/runc 凭证、profile 与 rootfs 制备；安装脚本补齐 Shell、Git、TLS 和 NSS 解析依赖。
