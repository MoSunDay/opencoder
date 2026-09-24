Commit: 48f6127cb5456123c4d083b871cce8b3f7fc527c

# brain 模块

能力目录与 schema_version 7 里程碑调度。`layers` 是有序的里程碑容器，分别持有目标和达成标准；`nodes` 通过 `layer_id` 归属一层，每个节点仅绑定一个泛化能力。`transitions` 限定层间出边，Brain 在整层并行执行终态后评估当前层，再从出边决定前进、回退或本层重试。回退开启新轮次，每次激活保留独立执行 ID；能力负责具体任务，大脑负责绑定输入、判断状态与选择下一层。

- `crates/core/src/brain/layered/`：计划、运行、操作与决策协议。
- `crates/core/src/brain/capability.rs`：能力描述及输入引用。
- `crates/brain/src/layered/`：图校验、分层、上下文、决策、终态、重试与命令纯函数。
- `crates/brain/src/runtime.rs`：能力库持久化及向量检索。
- `crates/control/src/api/brain_runs/plan_capabilities.rs`：保存计划能力化及嵌套引用校验。
- `crates/control/src/api/brain_runs/v4/`：准入、派发、事件转交及索引视图。
- `crates/worker/src/brain/v4/`：节点投影、恢复、模型调度与确认。
- `crates/brain/tests/layered/`：图约束、层屏障、终态；`crates/worker/tests/brain_nested.rs`：真实嵌套计划链路。

新计划和运行入口要求 schema 7；历史 schema 4/5/6 只读；`v4/` 是现存实现目录名，历史读取逻辑保留在代码中。历史数据清理使用 `scripts/maintenance/brain_cleanup/` 的审阅清单、行摘要校验、备份与重复复核；清理范围必须同时覆盖运行数据、Server 索引及 Host 休眠索引，避免节点同步恢复已删除的 ID。清理不在存储初始化中自动执行。

[运行协议](../../docs/brain-orchestration.md) · [工作台](../../features/brain/index.md)
