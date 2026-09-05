# agent — `opencoder-agent` 二进制（fleet worker）

`crates/agent`：节点工作进程入口（P0 三分叉拆分产物）。替代旧 `opencoder daemon --client`。构建本地 store/LLM client → `run_node(NodeOpts)`；默认同时挂 `DagRuntimeHook`（claim DAG run → `opencoder-dag-runtime::execute_run`，`--no-dag` 关闭）。子命令 `dag prepare-rootfs --out DIR` 离线生成 runc rootfs 脚手架（不需 remote/token）。

DAG claim 走节点 idle 轮询（单活跃 run 串行），取消经心跳应答的 `cancel_run_ids` 捎带。详见 [agents/node](../node/index.md)、[agents/dag-runtime](../dag-runtime/index.md)。
