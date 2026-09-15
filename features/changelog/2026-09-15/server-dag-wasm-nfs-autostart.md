Commit: c0b88d6131b493822219179b1135a0a01807822b

# opencoder-server 接入 DAG WASM NFS 自动启动

控制面已经具备 DAG WASM 制品池和第二路只读 NFS 导出，但
`opencoder-server` 启动流程此前只启动 Agent NFS，导致配置
`dag.nfs.enabled=true` 时 WASM 导出不会监听。

## 变更

- Server 启动时自动启动 DAG WASM NFS 导出，HTTP 控制面仍保持导出失败可用。
- Server 是制品池唯一写入方；节点以只读 NFS 加载资源。
- 节点受理时把 WASM 和 Agent 资源复制到本地快照后执行，任务不写 NFS。
- 补充控制面启动回归测试和部署说明。

## 验证

- `cargo test -p opencoder-control`
- `cargo test -p opencoder-agents`
- `cargo test -p opencoder-worker`
- `cargo test -p opencoder-web`
- `rustfmt --edition 2021 --check crates/control/src/bootstrap.rs`


## 线上生效

- 发布版本：`c0b88d61`（包含 `ea2052b2` 的 Server 自动启动逻辑）。
- Server `dag.nfs` 已启用，导出 `127.0.0.1:2050`，只读；节点通过 `/mnt/opencoder-dag-wasm` 持久化只读 NFS v3 挂载。
- 验证制品 `nfs-dag-e2e-20260915@v1.wasm`：Server 源池、NFS 挂载和节点本地 `_modules` SHA256 均为 `d716c85762bf57fb3f8e02f7be8856ca67a5bef41da2afaaebf54d0aadc17b39`；实际 DAG 执行完成，输出 `nfs-dag-e2e\n`。
- NFS 写入返回 `EROFS`；任务输出位于节点本地 DAG 工作目录。
