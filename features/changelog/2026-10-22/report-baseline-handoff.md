Commit: c53066e0c2181a21709c6edeb5de86b5a0ee61b9

# 共库索引基准变化时保留连接

发布中多个 Server 接收同一 Host 的报告，共享提交序号可能先被另一个 Server 推进。原实现关闭基准不一致的连接，会取消仍在等待的 RPC，并使公共入口短暂失去就绪节点。

连接现在只缓存五字段索引，普通报告按变化项更新；仅基准冲突时重建该连接的完整快照。存储仍核验代次、序号和不可变归属，不能直接把增量套在另一个连接的基准上。已经落后的报告不覆盖新状态，报告确认允许连接继续交付原 RPC 回复。协议、归属和实际存储错误仍拒绝。没有新增共享数据库结构或修改公开 API。

| 行为 | 定向验证 |
| --- | --- |
| 基准冲突、完整重建、RPC 连续和历史保留 | `shared_base_change_rebuilds_full_indexes_without_losing_pending_rpc_or_history` |
| 等待提交时不提前确认 | `final_delta_ack_waits_for_storage_and_brain_does_not_consume_fragment_sequence` |
| 新快照不被较旧重建覆盖，归属冲突仍拒绝 | `rebuilt_snapshot_respects_newer_data_and_still_validates_ownership` |
| 较旧报告确认与后续报告衔接 | `superseded_reports_acknowledge_without_overwriting_newer_durable_indexes` |
| 同一 Server 的 Host 接管后，旧连接确认且交付原 RPC | `locally_superseded_host_report_is_acknowledged_without_replacing_new_owner` |

上述五项验证已通过。最后一项先在真实 WebSocket 上复现确认超时，再验证修复保留原 RPC；后续三项增量用例使用该次已编译测试产物通过。控制面 transport 定向测试首轮 34 通过、2 项超时失败；两项按原阈值独立复测通过。最终独立快照的 Store 报告 5 项、Control 交接 3 项及两个 crate 的 Clippy 均通过。合并候选通过 17 个真实 runc 交接场景，232 次连续提交无失败，最大受理耗时 8.48 秒、最大调度间隔 17.53 秒；这不替代生产发布后的观察。

相关说明：[控制面](../../../agents/control/index.md)、[存储](../../../agents/store/index.md)、[原索引优化](cpu-index-sync.md)。

发布兼容检查区分保留历史和新候选：已有格式 11 的报告写入版本可被新读取器覆盖，但不能重新作为发布或回滚目标。新写入版本仍必须使用格式 12；旧读取器仍拒绝不能读取的历史。`test_historical_format_eleven_writer_is_readable_but_not_republished` 和报告格式的另 3 项用例通过，实际保留版本清单的只读兼容检查通过。

发布收尾也停用已经完成维护退役的 Server 登记，避免旧 Host 永远等待一个已停止 Server 的交接确认。该操作不停止 Runtime，也不删除历史。`test_maintenance_retired_server_no_longer_blocks_host_handoff`、`test_current_maintenance_retired_record_is_rejected` 及同文件其余 30 项发布定向测试通过。
