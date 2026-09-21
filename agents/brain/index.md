# brain 模块

能力目录与 schema_version 4 分层调度。计划由一句话节点、单一能力引用及连线构成；层级纯函数推导，整层成功越过屏障，重试耗尽失败并取消兄弟。执行与验证由能力自身负责。

- `crates/core/src/brain/layered/`：计划、运行、操作与决策协议。
- `crates/core/src/brain/capability.rs`：能力描述及输入引用。
- `crates/brain/src/layered/`：图校验、分层、上下文、决策、终态、重试与命令纯函数。
- `crates/brain/src/runtime.rs`：能力库持久化及向量检索。
- `crates/control/src/api/brain_runs/plan_capabilities.rs`：保存计划能力化及嵌套引用校验。
- `crates/control/src/api/brain_runs/v4/`：准入、派发、事件转交及索引视图。
- `crates/worker/src/brain/v4/`：节点投影、恢复、模型调度与确认。
- `crates/brain/tests/layered/`：图约束、层屏障、终态；`crates/worker/tests/brain_nested.rs`：真实嵌套计划链路。

旧决策树、playbook、v2/v3 内核已删除；历史数据的清理必须先核准范围，不在存储初始化中自动执行。

[运行协议](../../docs/brain-orchestration.md) · [工作台](../../features/brain/index.md)
