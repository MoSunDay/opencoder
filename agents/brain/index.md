Commit: d26f8cb5a16a52072daad02c77ae63161973526b

# brain 模块

能力目录与 schema_version 6 里程碑调度。计划用显式层级组织里程碑，每个里程碑定义名称、目标、达成标准及 1–32 个能力引用。层内能力执行结束后形成屏障，大脑评估当前结果，决定推进下一层或反思回退到先前层；回退开启新轮次，每次激活产生独立的能力执行 ID。能力负责具体执行，大脑负责输入绑定、状态评估与下一步决策。

- `crates/core/src/brain/layered/`：计划、运行、操作与决策协议。
- `crates/core/src/brain/capability.rs`：能力描述及输入引用。
- `crates/brain/src/layered/`：图校验、分层、上下文、决策、终态、重试与命令纯函数。
- `crates/brain/src/runtime.rs`：能力库持久化及向量检索。
- `crates/control/src/api/brain_runs/plan_capabilities.rs`：保存计划能力化及嵌套引用校验。
- `crates/control/src/api/brain_runs/v4/`：准入、派发、事件转交及索引视图。
- `crates/worker/src/brain/v4/`：节点投影、恢复、模型调度与确认。
- `crates/brain/tests/layered/`：图约束、层屏障、终态；`crates/worker/tests/brain_nested.rs`：真实嵌套计划链路。

新计划和运行入口要求 schema 6；`v4/` 是现存实现目录名，历史读取逻辑保留在代码中。历史数据清理使用 `scripts/maintenance/brain_cleanup/` 的审阅清单、行摘要校验、备份与重复复核；清理范围必须同时覆盖运行数据、Server 索引及 Host 休眠索引，避免节点同步恢复已删除的 ID。清理不在存储初始化中自动执行。

[运行协议](../../docs/brain-orchestration.md) · [工作台](../../features/brain/index.md)
