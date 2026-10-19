Commit: 33d91d7cd47127fbf15b8d38a1b4a62ac08d9c8e

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

Windows 的 `opencoder-agent --remote` 提供原生 Operator 节点，启动前要求 PowerShell 7；不提供 Windows 服务、Server 或滚动发布。安装与运行见 [Windows](../../features/windows/index.md)。以下 Host 发布机制用于现有 Linux 部署。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `crates/agent/src/host/service.rs` — 节点级维护命令中继
- `crates/agent/src/host/mod.rs` — fleet channel 装配
- [host/lifecycle.rs](../../crates/agent/src/host/lifecycle.rs) — 当前 Host 每 5 秒回收 retired Runtime；要求无 reservation 且 inventory 允许休眠，staged Runtime 不在自动回收范围。[最近访问记录](../../crates/agent/src/host/lifecycle/activity.rs) 让实际转发过 RPC 的 Runtime 在请求结束后保留 60 秒，连续读取历史时不反复休眠；自动回收取得使用锁后再次核对当前 Host 身份与最近访问。Host 直接读取持久化结果时不延长 Runtime 存活时间，显式休眠仍使用原有安全条件。
- [host/mod.rs](../../crates/agent/src/host/mod.rs) — Host 曾在本进程中成为 current，且后继完成 ingress 与 Server ACK 后才进入退出流程；从未激活的 standby 不走这一退出分支。
- [host/runtime.rs](../../crates/agent/src/host/runtime.rs) — Runtime inventory 提供执行、子进程及可休眠状态；排查同名进程时区分 Host、Runtime、节点及 internal-process-supervisor。

## 报告与发布边界

- [host/client.rs](../../crates/agent/src/host/client.rs)：普通只读 RPC 不增加库存修订号；唤醒 Runtime 或清除休眠标记仍通知同步。Host 汇总全部 Runtime 索引，但只用活动 Runtime 的健康状态决定新任务准入；退休 Runtime 的休眠快照错误不阻断当前版本。
- 已确认休眠的 retired Runtime 直接读取冻结的最终清单，不探测旧端口、不清除休眠标记。实际唤醒按原使用锁与准入流程清除标记后恢复实时读取；未休眠版本清单不可用仍报告未就绪，不能使用缓存掩盖错误。
- Runtime 在准入前返回的 HTTP 422 保持为明确拒绝，不能包装成可重试的 503；执行归属保持原 Runtime，转发结束释放使用锁。回归见 [host/tests/forwarding.rs](../../crates/agent/src/host/tests/forwarding.rs)。
- [host/client.rs](../../crates/agent/src/host/client.rs) 对 `result`、`team.topic` 的分块读取复用 [result_reader](../../crates/worker/src/result_reader/mod.rs)，在原执行所属 Runtime 的目录只读读取，补齐内容版本，不唤醒、恢复或迁移原执行。其他请求仍按执行归属转发；升级顺序见 [项目模块约定](../../rules/07-project-module-contract.md)。
- [host/service.rs](../../crates/agent/src/host/service.rs)、[host/client.rs](../../crates/agent/src/host/client.rs)：节点冻结、查询准入和重新开放跳过已休眠 Runtime；需要转发给 Runtime 的请求若触发唤醒，先依次取得 Host 准入锁和 Runtime 使用锁，再同步当前准入模式，成功后清除休眠标记并转发请求。
- [host/mod.rs](../../crates/agent/src/host/mod.rs)：关闭信号 future 在主循环外固定，库存和通道变化不会丢弃已到达的 SIGTERM。
- [rolling/deployment.py](../../scripts/platform/rolling/deployment.py)：历史 Server/Host 批量停用后统一重载 systemd；不停止保留 Runtime。
- [rolling/probes.py](../../scripts/platform/rolling/probes.py)：切换前优先检查候选 Server 上的就绪节点；旧 Host 因退休 Runtime 状态不就绪时，改为核验已完成执行探针的候选 Runtime、候选 Host 与候选 Server 身份，切换后仍执行公共入口探针。
- [rolling/backup.py](../../scripts/platform/rolling/backup.py)：单次遍历各根目录，最多并行处理四个根，包含嵌套数据库且不跟随资源符号链接；逐库固定 WAL 读取快照并核验完整性，全部成功后原子发布。[network/ports.py](../../scripts/platform/rolling/network/ports.py) 探测整组端口可绑定性。
- [rolling/native](../../scripts/platform/rolling/native/__init__.py)：每个 Runtime 使用本版本私有镜像与固定配置，校验已发布副本后才复用。Runtime unit 通过 `unshare --mount --propagation private` 运行，防止 DAG 挂载传播到其他服务；不依赖会遮蔽资源服务进程的 systemd `PrivateMounts`。
- [rolling/config.py](../../scripts/platform/rolling/config.py)、[rolling/storage](../../scripts/platform/rolling/storage/__init__.py)：版本部署只声明持久化根；Host、Runtime、工作目录、镜像和 NFS 客户端路径由同一布局派生。独立启动的 `--data-dir` 接口保持独立使用，外部 DAG 源工作区不属于受管状态。定向真实容器及切换验收见 [storage_root](../../scripts/acceptance/storage_root/main.py)。

原生 DAG 约定见 [规则 04](../../rules/04-dag-execution-contract.md)，发布与维护边界见 [平滑发布](../../docs/smooth-release.md)。

Host 报告计时分别覆盖 report_gate、全部 Runtime 清单汇总和 Brain 帧收集，只记录节点、条数与毫秒值；慢同步不能通过跳过旧 Runtime 清单、改变执行 owner 或缓存未确认的活跃清单来标记就绪。

Host 在创建与能力探测时使用相同的父 Runtime 归属。共享工作区按 `brain_layered.run_id` 查找本机父归属，嵌套 Brain 使用 `layered_request.parent`；普通子执行若父在本机则继承，若父在其他节点则保持该节点的独立选择。探测不保存子执行 owner，已接受执行始终保留原 owner。Worker 继续核对原生父运行与 operation 身份。

历史读取验收使用 [history/retired.py](../../scripts/acceptance/brain/history/retired.py) 固定执行与退役 Runtime。每次读前后核对同一归属和配置，读取后记录真实 PID、启动时间与可执行文件；归属缺失、指向当前 Runtime、读取失败或进程身份不完整都不能通过。观察期间固定这些证据，不能替换历史样本。
