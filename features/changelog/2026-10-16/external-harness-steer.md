Commit: a388f28a2e631e962c01d554419cd4fc90476078

# 外部 Harness 同轮次输入

外部启动脚本原来只收到整轮提示词，steer 会结束本地观察器并作为下一轮输入。可选通用控制协议现在把实时输入交给独立受管调用，保留原观察器、线程、父输入和稳定输入身份。

拒绝与不确定结果分别保留；恢复先核对原请求，避免已发送输入变成普通 followup。普通 Codex 保持原行为，queue 仍等待轮次边界。控制正文用 stdin，不塞入环境变量。

## 功能与测试

`crates/session/tests/harness_control.rs`：

- 即时输入、重复身份与队列：`steer_keeps_observer_and_queue_waits_for_next_turn`
- 结束竞争拒绝：`ended_turn_rejection_is_visible_and_never_becomes_followup`
- 未知回执：`uncertain_control_reconciles_original_identity_without_duplicate_effect`
- 取消归属：`hard_cancel_uses_original_external_input_before_local_cleanup`
- 持久恢复：`restart_reconciles_orphaned_control_without_resending_as_followup`
- 空闲超时：`control_polling_does_not_extend_a_dead_observers_idle_timeout`

同时回归 `harness_codex`、`harness_startup`、`harness_stack`；测试范围仅本次修改与直接影响链路。发布、真实外部实现和观察回执分别记录，单元通过不等于部署验收。

参见[协议](../../../docs/external-harness-control.md)、[会话模块](../../../agents/session/index.md)、[Harness](../../harness/index.md)。
