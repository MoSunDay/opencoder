Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# brain 模块

能力目录与分层里程碑调度：能力负责具体任务，大脑负责绑定输入、
判断状态与选择下一层。

## 索引
- `crates/core/src/brain/layered/` — 里程碑计划、运行、操作与决策协议
- `crates/core/src/brain/capability.rs` — 能力描述符与证据容量常量
- `crates/brain/src/contracts.rs` — 能力字段与决策证据的纯校验
- `crates/brain/src/layered/` — 图校验、分层、上下文、决策、终态与重试
- `crates/brain/src/runtime.rs` — 能力库持久化与向量检索
- `crates/control/src/api/brain_runs/plan_capabilities.rs` — 计划能力化保存
- `crates/control/src/api/brain_runs/v4/` — 准入、派发、事件转交与视图
- `crates/control/src/api/brain_runs/catalog/availability.rs` —
  profile 引用可用性检查
- `crates/worker/src/brain/v4/` — 节点侧投影、恢复、模型调度与确认
- `crates/worker/src/brain/v4/budget.rs` — 冻结模型容量与人工输入准入
- `crates/worker/src/brain/workspace/mod.rs` — 每 run 独立工作区
- `crates/worker/src/brain/output/index.rs` — 内容寻址的输出清单索引

## 测试与运维
- `crates/brain/tests/milestone.rs`、`milestone/` — 图约束与层屏障
- `crates/worker/tests/brain_scheduler_v4.rs` — 节点调度回归
- `crates/worker/tests/brain_nested.rs` — 嵌套计划链路回归
- `crates/worker/tests/brain_io/main.rs` — 输入输出引用全链路回归
- `crates/worker/tests/brain_closed_loop.rs` — 原生闭环与版本传递回归
- `crates/worker/tests/brain_contracts.rs`、`brain_dispatch_failures.rs` —
  输出契约与拒绝回执回归
- `scripts/ci/brain.py` — CI 入口
- `scripts/acceptance/brain/README.md` — 验收说明
- `scripts/maintenance/brain_cleanup/` — 历史数据清理维护脚本

## 相关
[调度规则](../../../rules/06-brain-scheduling-contract.md) ·
[运行协议](../../../crates/control/src/api/brain_runs) ·
[工作台](../../../features/platform/brain/index.md)
