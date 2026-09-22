Commit: 1afd5d4375cd10885aee335d3d9dbf9d396bb563

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `crates/agent/src/host/service.rs` — 节点级维护命令中继
- `crates/agent/src/host/mod.rs` — fleet channel 装配
- [host/lifecycle.rs](../../crates/agent/src/host/lifecycle.rs) — 当前 Host 每 5 秒回收 retired Runtime；要求无 reservation 且 inventory 允许休眠，staged Runtime 不在自动回收范围。
- [host/mod.rs](../../crates/agent/src/host/mod.rs) — Host 曾在本进程中成为 current，且后继完成 ingress 与 Server ACK 后才进入退出流程；从未激活的 standby 不走这一退出分支。
- [host/runtime.rs](../../crates/agent/src/host/runtime.rs) — Runtime inventory 提供执行、子进程及可休眠状态；排查同名进程时区分 Host、Runtime、节点及 internal-process-supervisor。

## 相关
- 陷阱案例：[release-0662b924-signal-deploy](../../features/changelog/2026-09-18/release-0662b924-signal-deploy.md)

## 报告与发布边界

- [host/client.rs](../../crates/agent/src/host/client.rs)：普通只读 RPC 不增加库存修订号；唤醒 Runtime 或清除休眠标记仍通知同步。
- [rolling/deployment.py](../../scripts/platform/rolling/deployment.py)：历史 Server/Host 批量停用后统一重载 systemd；不停止保留 Runtime。
- [rolling/backup.py](../../scripts/platform/rolling/backup.py)、[network/ports.py](../../scripts/platform/rolling/network/ports.py)：在线备份固定各数据库的 WAL 读取快照，端口分配探测整组可绑定性。
