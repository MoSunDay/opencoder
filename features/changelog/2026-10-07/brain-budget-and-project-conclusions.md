Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 大脑上下文容量准入

多节点计划在执行前按完整证据上限校验上下文容量，并冻结有效模型窗口。人工输入按累计容量校验，实际决策前再次检查；超限返回 413 或明确阻塞，不先派发子执行。

节点须提供 `brain_context_budget_v1`。默认模型窗口仍为 128000 tokens；多节点调度测试显式配置更大的模拟模型窗口，不放宽生产容量检查。没有新增数据库表或产品环境变量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 宽计划、转义、嵌套证据容量 | `valid_wide_plan_is_rejected_before_any_child_can_run`、`model_capacity_and_json_escaping_are_included_in_admission`、`nested_receipts_and_large_frozen_definitions_are_reserved` | [context_budget.rs](../../../crates/brain/tests/context_budget.rs) |
| 节点准入与人工输入无副作用拒绝 | `probe_and_create_reject_a_plan_before_execution_admission`、`rejected_human_input_does_not_append_events_or_advance_generation` | [brain_context_budget.rs](../../../crates/worker/tests/brain_context_budget.rs) |
| 服务端保留 413 且不创建索引 | `node_capacity_rejection_is_reported_before_creating_a_brain_index` | [layered_api/mod.rs](../../../crates/control/tests/e2e/layered_api/mod.rs) |

本次合并的完整验证见 2026-10-09 仓库清理记录。项目结果采用用户确认的实时读取方案。

相关：[大脑调度规则](../../../rules/06-brain-scheduling-contract.md)、[大脑工作台](../../brain/index.md)。
