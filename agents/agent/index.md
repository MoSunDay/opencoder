Commit: 86fa2372caffd649b5809d14ca410fb269dcb760

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

Windows 的 `opencoder-agent --remote` 提供原生 Operator 节点，启动前要求 PowerShell 7；不提供 Windows 服务、Server 或滚动发布。安装与运行见 [Windows](../../features/windows/index.md)。以下 Host 发布机制用于现有 Linux 部署。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `crates/agent/src/host/service.rs` — 节点级维护命令中继
- `crates/agent/src/host/mod.rs` — fleet channel 装配；Host 周期采集 500 毫秒（远低于 30 秒调度时延预算）
- [host/lifecycle.rs](../../crates/agent/src/host/lifecycle.rs) — 当前 Host 每 5 秒回收 retired Runtime；要求无 reservation 且 inventory 允许休眠，staged Runtime 不在自动回收范围。[最近访问记录](../../crates/agent/src/host/lifecycle/activity.rs) 让实际转发过 RPC 的 Runtime 在请求结束后保留 60 秒，连续读取历史时不反复休眠；自动回收取得使用锁后再次核对当前 Host 身份与最近访问。Host 直接读取持久化结果时不延长 Runtime 存活时间，显式休眠仍使用原有安全条件。
- [host/mod.rs](../../crates/agent/src/host/mod.rs) — Host 曾在本进程中成为 current，且后继完成 ingress 与 Server ACK 后才进入退出流程；从未激活的 standby 不走这一退出分支。
- [host/runtime.rs](../../crates/agent/src/host/runtime.rs) — Runtime inventory 提供执行、子进程及可休眠状态；排查同名进程时区分 Host、Runtime、节点及 internal-process-supervisor。

## Runtime 工作线程

[executor.rs](../../crates/agent/src/executor.rs) 将 Runtime、兼容 Run 与默认执行入口的 Tokio 工作线程设为可用 CPU 数与 8 的较小值。较少线程减少大量短任务在不同线程分配后留下的空闲堆内存；`TOKIO_WORKER_THREADS` 显式配置继续交给 Tokio 校验和使用。Host 和离线工具保留原默认值。该线程池负责轮询异步任务，不改变 `max_runs`、DAG 步骤并发、Brain 同层并行或子进程额度。

## 报告与发布边界

- [host/client.rs](../../crates/agent/src/host/client.rs)：普通只读 RPC 不增加库存修订号；唤醒 Runtime 或清除休眠标记仍通知同步。Host 汇总全部 Runtime 索引，但只用活动 Runtime 的健康状态决定新任务准入；退休 Runtime 的休眠快照错误不阻断当前版本。
- 已确认休眠的 retired Runtime 直接读取冻结的最终清单，不探测旧端口、不清除休眠标记。实际唤醒按原使用锁与准入流程清除标记后恢复实时读取；未休眠版本清单不可用仍报告未就绪，不能使用缓存掩盖错误。
- Runtime 在准入前返回的 HTTP 422 保持为明确拒绝，不能包装成可重试的 503；执行归属保持原 Runtime，转发结束释放使用锁。回归见 [host/tests/forwarding.rs](../../crates/agent/src/host/tests/forwarding.rs)。
- [host/client.rs](../../crates/agent/src/host/client.rs) 对 `result`、`team.topic` 的分块读取复用 [result_reader](../../crates/worker/src/result_reader/mod.rs)，在原执行所属 Runtime 的目录只读读取，补齐内容版本，不唤醒、恢复或迁移原执行。其他请求仍按执行归属转发；升级顺序见 [项目模块约定](../../rules/07-project-module-contract.md)。
- [host/inventory/](../../crates/agent/src/host/inventory/mod.rs) 按每个 Runtime 的 generation 和 revision 获取变化。首次、重连、游标过期及每 10 分钟校准接收固定分页快照；旧 Runtime 的 404/405 响应使用原清单接口。Host 缓存执行摘要，先核验身份和不可变归属再合入变化；失败不发布半份清单，下次重新全量核对。已休眠 Runtime 只查询小的修订标记，冻结清单变化时才重新解析。

[host/service.rs](../../crates/agent/src/host/service.rs) 的 `index_changes` 将源变化直接交给 Node，同步后仍单独采样实时负载。旧 `index_report` 返回缓存中的完整摘要。Brain 单独收集，只有旧对端的 `report` 保留覆盖索引与 Brain 的整份报告锁。

Host 的本地 `/status` 返回 `index_report_version` 与 `reported_servers`。后者只包含启用且已经确认当前 Host 实例完整报告的 Server；旧实例的确认不能用于新实例。控制器在接管前检查报告协议 2，接管后等待当前 Server 的确认，再登记候选 Server。此阶段保持原 Runtime 和 Server 任务入口，Host 管理入口跟随已接管的 Host；旧 Runtime 的已确认故障可继续进入修复预热，Host 自身故障仍拒绝。
