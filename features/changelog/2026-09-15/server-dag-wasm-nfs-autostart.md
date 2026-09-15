Commit: 6977bde19a831a804e1a785ae24596f4c304e87d

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
