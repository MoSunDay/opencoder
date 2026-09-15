Commit: 1ac64fe8b81a2c7c144c72b717a8031ab18f2589

# agent 模块

`opencoder-agent` 提供稳定 Host、独立版本 Runtime，以及兼容单进程节点入口。

## 关键路径

- `crates/agent/src/main.rs` — `host`、`runtime`、默认 `run`、容器内部进程监督、DAG 镜像准备和本地存储迁移。
- `crates/agent/src/host/mod.rs`、`api.rs` — Host 使用稳定 Node ID，持久化 Runtime 注册与激活指针，按执行归属路由。
- `crates/agent/src/host/service.rs` — 聚合 Runtime 索引、节点快照和 Brain 上行帧，实现 NodeService。
- `crates/agent/src/host/client.rs` — Runtime 本地 RPC；历史访问通过独立 systemd unit 唤醒原版本。
- `crates/agent/src/host/runtime.rs` — 仅监听本机的认证 `/inventory`、`/rpc`、`/frames`，Worker 持有执行内容。
- `crates/agent/src/host/lifecycle.rs` — Host 交接确认、无任务/队列/工具/写入后的 Runtime 休眠。

## 边界

- Host 不持有任务进程；Runtime 的 unit、目录、锁与 runtime.db 独立，发布不向执行发送中断信号。
- Host 通过递增交接编号和各 Server 的索引确认完成主备切换；旧实例等在途请求完成后退出。
- Runtime 启动时固定全局技能；Host 不播种技能。整机容量与 FIFO 由共享 Host 账本管理，内部子会话保持父执行归属。
- Runtime 收到终止信号时先证明可休眠；有任务、队列或工具时拒绝退出。兼容 `run` 模式仍使用原有有界 drain 逻辑。
- 凭证来自既有参数、文件或配置，不自动生成。wasmtime/runc 执行依赖只在节点侧。

## 相关

- [worker](../worker/index.md) — 执行与恢复
- [node](../node/index.md) — 出站通道
- [store](../store/index.md) — 归属与容量账本
- [平滑发布](../../docs/smooth-release.md) — 部署协议
