Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

Windows 的 `opencoder-agent --remote` 提供原生 Operator 节点，启动前要求 PowerShell 7；不提供 Windows 服务、Server 或滚动发布。安装与运行见 [Windows](../../features/windows/index.md)。以下 Host 发布机制用于现有 Linux 部署。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `crates/agent/src/host/service.rs` — 节点级维护命令中继
- `crates/agent/src/host/mod.rs` — fleet channel 装配
- [host/lifecycle.rs](../../crates/agent/src/host/lifecycle.rs) — 当前 Host 每 5 秒回收 retired Runtime；要求无 reservation 且 inventory 允许休眠，staged Runtime 不在自动回收范围。
- [host/mod.rs](../../crates/agent/src/host/mod.rs) — Host 曾在本进程中成为 current，且后继完成 ingress 与 Server ACK 后才进入退出流程；从未激活的 standby 不走这一退出分支。
- [host/runtime.rs](../../crates/agent/src/host/runtime.rs) — Runtime inventory 提供执行、子进程及可休眠状态；排查同名进程时区分 Host、Runtime、节点及 internal-process-supervisor。

## 报告与发布边界

- [host/client.rs](../../crates/agent/src/host/client.rs)：普通只读 RPC 不增加库存修订号；唤醒 Runtime 或清除休眠标记仍通知同步。Host 汇总全部 Runtime 索引，但只用活动 Runtime 的健康状态决定新任务准入；退休 Runtime 的休眠快照错误不阻断当前版本。
- [host/client.rs](../../crates/agent/src/host/client.rs) 对 `result`、`team.topic` 的分块读取复用 [result_reader](../../crates/worker/src/result_reader/mod.rs)，在原执行所属 Runtime 的目录只读读取，补齐内容版本，不唤醒、恢复或迁移原执行。其他请求仍按执行归属转发；升级顺序见 [项目模块约定](../../rules/07-project-module-contract.md)。
- [host/service.rs](../../crates/agent/src/host/service.rs)、[host/client.rs](../../crates/agent/src/host/client.rs)：节点冻结、查询准入和重新开放跳过已休眠 Runtime；需要转发给 Runtime 的请求若触发唤醒，先依次取得 Host 准入锁和 Runtime 使用锁，再同步当前准入模式，成功后清除休眠标记并转发请求。
- [host/mod.rs](../../crates/agent/src/host/mod.rs)：关闭信号 future 在主循环外固定，库存和通道变化不会丢弃已到达的 SIGTERM。
- [rolling/deployment.py](../../scripts/platform/rolling/deployment.py)：历史 Server/Host 批量停用后统一重载 systemd；不停止保留 Runtime。
- [rolling/probes.py](../../scripts/platform/rolling/probes.py)：切换前优先检查候选 Server 上的就绪节点；旧 Host 因退休 Runtime 状态不就绪时，改为核验已完成执行探针的候选 Runtime、候选 Host 与候选 Server 身份，切换后仍执行公共入口探针。
- [rolling/backup.py](../../scripts/platform/rolling/backup.py)、[network/ports.py](../../scripts/platform/rolling/network/ports.py)：在线备份固定各数据库的 WAL 读取快照，端口分配探测整组可绑定性。
- [rolling/native](../../scripts/platform/rolling/native/__init__.py)：每个 Runtime 使用本版本私有镜像与固定配置，校验已发布副本后才复用。Runtime unit 通过 `unshare --mount --propagation private` 运行，防止 DAG 挂载传播到其他服务；不依赖会遮蔽资源服务进程的 systemd `PrivateMounts`。

原生 DAG 约定见 [规则 04](../../rules/04-dag-execution-contract.md)，发布与维护边界见 [平滑发布](../../docs/smooth-release.md)。
