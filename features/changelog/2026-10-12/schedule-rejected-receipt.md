Commit: 2863942fb0888a7caa6a22220d695d26759858cd

# 定时任务在明确拒绝后继续后续周期

定时任务在 NFS 短暂异常时被拒绝提交。挂载恢复后，cron 仍在一小时重试窗口内反复提交相同 ID，命中持久化拒绝回执，导致后续周期无法执行。

调度器现在读取失败任务的派发回执：`rejected` 保留原失败台账并进入正常 cron 时间计算；未确定或临时失败继续沿原 ID 重试。相同 ID 的拒绝回放、冻结输入、任务归属及只读 NFS 校验不变。Brain 的独立提交入口不读取普通执行回执。

验证：调度时间单测 9 项、`schedule_api` HTTP 集成测试 17 项、拒绝回执幂等回放 1 项通过；`opencoder-control` 的 lib 与 e2e 目标 Clippy 无警告。新增集成测试先在修复前复现后续周期被阻塞，并覆盖 503 恢复时重试原 ID；时间单测覆盖下个周期前不提前触发及停机追赶。发布验收脚本的 15 项定向测试通过，未运行全量测试。

关联：[调度实现](../../../crates/control/src/scheduler.rs)、[回归测试](../../../crates/control/tests/e2e/schedule_api/retry.rs)、[平台行为](../../agent-platform/index.md)。
