Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# 大脑按能力结果完成编码与验证闭环

输入引用失败不再停留于 Creating：不可重试的拒绝与原因随根操作终态持久化，重新唤醒大脑。决策读取完整结构化结果，避免只看到 `summary` 而遗漏验证失败。能力描述、必填输入输出字段随执行定义冻结；字段约定复用现有目标绑定存储，不新增表或环境变量。

能力库表单保存字段要求及默认 Agent 绑定。DAG 实际执行读取 Brain 绑定的参数；能力之间显式传递代码版本、源码或产物。超限或缺失输出明确失败，原始结果保留在子执行。模型纠错的已过期时限直接结束，避免有符号时间差转为超大等待时间。

## 功能与测试

| 功能 | 测试入口与用例 |
| --- | --- |
| 输入引用错误、真实测试失败、修复后通过，并核对代码版本 | [brain_closed_loop.rs](../../../crates/worker/tests/brain_closed_loop.rs)：`native_coding_test_loop_recovers_binding_errors_and_task_failures` |
| 失败布尔值完整传递、必填输出缺失、大结果明确拒绝 | [brain_contracts.rs](../../../crates/worker/tests/brain_contracts.rs)：三个输出集成用例；[contracts.rs](../../../crates/brain/src/contracts.rs)：字段与大小纯函数测试 |
| 派发拒绝携带原因、临时失败重试、重复回执幂等 | [brain_dispatch_failures.rs](../../../crates/worker/tests/brain_dispatch_failures.rs)：`rejected_dispatch_keeps_diagnostics_and_replay_cannot_duplicate_the_failure` |
| 字段要求保存、回读并进入能力目录 | [brain_api/contracts.rs](../../../crates/control/tests/e2e/brain_api/contracts.rs)：`capability_field_requirements_persist_in_target_and_reach_the_scheduler_catalog` |
| 描述送到执行器，DAG 收到准确参数，过期时限不回绕 | [gateway.rs](../../../crates/control/src/api/brain_runs/v4/gateway.rs)、[workloads/dag.rs](../../../crates/worker/src/workloads/dag.rs)、[correction.rs](../../../crates/worker/src/brain/v4/correction.rs) 的对应单测 |
| 表单默认绑定与字段编辑；真实浏览器保存、回读、失败后重试 | [model.test.js](../../../crates/web/spa/src/brain/model.test.js)、[brainPanel.dom.test.jsx](../../../crates/web/spa/src/brainPanel.dom.test.jsx)、[capability-contracts.js](../../../scripts/acceptance/brain/capability-contracts.js) |
| 不把新任务交给缺少字段校验能力的旧节点 | [layered_api/mod.rs](../../../crates/control/tests/e2e/layered_api/mod.rs)：`layered_admission_requires_the_v4_advertisement_and_freezes_the_scope` |

## 验证与边界

- Rust 全量 5,668 项通过；默认跳过 7 个手动项，其中 Brain 浏览器用例已显式执行。全量测试进程使用 8 路并发及 65,536 文件数量上限，避免高核数主机的默认并发耗尽文件描述符。
- SPA 全量 966 项、格式检查、Clippy 零警告、workspace 完整构建通过。全站四种屏宽、功能链路及 TUI 验收通过。
- 真实 runc 调度、重启恢复、取消与只读源校验通过，持续观察 15 分钟；三 Runtime 切换、回滚、存量任务连续性及 15 分钟观察通过。模型回答使用确定性测试实现。
- 新根运行和托管子执行要求节点声明 `brain_contracts_v1`。输入输出约定不包含任务通过条件；大脑仍按里程碑达成标准判断。计划范围、整层屏障和轮次预算继续生效。
- 本轮验证使用独立开发构建与镜像；未提交代码，未生成或部署正式发布包。

[调度逻辑](../../../agents/brain/index.md) · [工作台行为](../../brain/index.md)
