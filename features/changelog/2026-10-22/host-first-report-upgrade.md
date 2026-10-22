Commit: 842e58ef5525e8f189895ad0cc7ac59031f312fc

# 先接管 Host，再预热共享数据库的 Server

真实跨版本演练中，候选 Server 推进了共享索引序号，旧 Server 随后拒绝其连接上的增量基准并断开，连续提交出现一次 503。仅修复新 Server 的完整快照重建，不能保护已经运行的旧 Server。

发布和回滚现在先核验新 Host 支持报告协议 2，在原 Server 上完成接管及完整报告确认，再登记候选 Server。原 Runtime 与 Server 任务入口在预热期间保持原版本；失败或重复执行沿用同一实例，切换前仍检查目标代次与完整就绪。当前实例的报告确认不能由旧 Host 留下的确认代替。旧 Runtime 的明确故障允许修复预热，未知 Host 错误仍拒绝。

新 Host 接管后同步更新固定 Host 管理入口，避免旧 Host 拒绝发布重试信号。仅改写 Host 上游，保留实际 Server 上游；不以可能已提前变化的发布记录推断当前任务入口。生产配置的临时副本验证通过，生产入口未改动。

增量报告使用版本 2 协商。新 Host 对旧 Server 发送带逐批确认的全量报告；新 Server 继续保留连接内五字段基准。全量报告中的 Brain 帧不消耗索引确认序号，避免等待 Server 不会发送的确认。数据结构、鉴权和执行归属不变，没有新增用户能力或公共入口，既有 Ontology 能力登记仍适用。

| 行为 | 定向验证 |
| --- | --- |
| 旧 Server 收到完整变化，Brain 消息不阻塞确认 | `legacy_server_receives_acknowledged_full_reports_after_changes` |
| 新协议保持增量、RPC、Brain 与提交确认 | `delta_waits_for_commit_while_brain_and_rpc_continue_and_idle_has_no_report` |
| 新实例不能复用旧实例或已停用 Server 的报告确认 | `status_proves_only_this_hosts_enabled_server_report_acknowledgements` |
| 先完成生产者接管，再登记候选；失败保持原入口 | `test_new_host_is_ready_on_current_server_before_candidate_registration`、`test_failed_host_prewarm_cannot_register_candidate_or_switch_runtime` |
| 拒绝缺少协议 2 的候选生产者 | `test_old_report_producer_is_rejected_before_host_activation` |
| 候选预热失败后仍指向已接管 Host | `test_failed_candidate_prewarm_keeps_management_on_the_new_host` |
| 仅更新 Host 上游、重复执行不重载、失败恢复原配置 | `test_host_switch_preserves_actual_server_and_retry_does_not_reload`、`test_invalid_layout_and_failed_reload_preserve_original_config` |
| 完整报告确认、故障归因与切换就绪分别验证 | `test_producer_prewarm_requires_its_report_ack_but_allows_known_runtime_fault` |

发布控制器 38 项、报告探针 16 项与 Node 两项真实 WebSocket 测试通过。Control 11 项集成检查中，首次并行运行有 4 项超时；原测试程序逐项复核全部通过，断言和超时未调整，原失败日志保留。另有 3 项交接检查、1 项 Host 实例确认检查及相关 crate 的 Clippy 通过。只执行改动及直接影响链路的定向测试。

最终原生程序按实际部署版本 `8a4025fa` → 保护回滚版本 `225053db` → 内存候选 `f55dfb36` → `225053db` 演练，发布控制器来自 `842e58ef`，内容摘要为 `58f50fb5eb7ca4df49e2fa3e19dca214ed8a82db495aad388bdd87ce72ae2ebf`。全部 18 项检查通过，包含任务连续性、真实容器执行、崩溃恢复、历史唤醒、SSE 恢复、数据保留和稳定观察。208 次连续提交零失败，最大受理耗时 2.20 秒，最大调度间隔 3.81 秒，最大调度延迟 18.11 秒。

所有交接样本结束后，候选与回滚版本各持续观察 900 秒，分别完成 53、54 个任务，Runtime 未重启。实际进程与容器内程序摘要逐项核验，测试数据与线上 Runtime 位于同一块 ext4 磁盘。两套程序包和独立控制器包均通过归档回读；线上基线、写入权限、数据库结构和容量预检通过。先激活保护回滚版本，再激活内存候选；保留任务仍由原 Runtime 执行。本次完成隔离验收与上线准备，未激活生产版本。

相关说明：[Node](../../../agents/node/index.md)、[Host](../../../agents/agent/index.md)、[发布流程](../../../docs/smooth-release.md)、[共享基准重建](report-baseline-handoff.md)。
