Commit: 4bb3a745544f3b3f9898447088a919da502de6e5

# brain 模块

能力目录与 schema_version 7 里程碑调度。`layers` 是有序的里程碑容器，必填名称、任务、目标和达成标准；`nodes` 通过 `layer_id` 归属一层，每个节点仅绑定一个泛化能力，节点标题与任务由所选能力生成。Brain 在整层并行执行终态后评估当前层：达标后前进到紧邻下一层，需整改时可选择本层或任一已执行层；旧 `transitions` 可读取，但不参与模型决策。新计划在保存和运行准入时由纯函数补齐内部前进与回退路径，兼容现有运行图校验与不可变重试，用户无需配置这些路径。回退开启新轮次，每次激活保留独立执行 ID；能力负责具体任务，大脑负责绑定输入、判断状态与选择下一层。所有能力使用相同的决策校验，不按业务名称增加专用限制。

运行创建、层屏障和人工输入是决策唤醒事件。人工输入持久化后令当前决策失效并重新组装上下文；层屏障未满足时只允许 `guide`，不能派发下一层。`guide` 可向当前运行中的 Agent/Operator 投递 steer，或让 Team 在下一次成员发问时应用引导，投递事件按序确认并可重放。生产模型激活经 OpenCoder session agent loop，每次唤醒只产生一个有限决策。

调度范围是计划已绑定的能力，每次派发覆盖目标层全部节点；运行中不检索能力库来增加节点或替换能力。能力描述、输入输出描述及必填字段随能力定义冻结，进入调度与执行提示词。[字段校验](../../crates/brain/src/contracts.rs) 负责字段名、输入值和输出证据的纯函数校验；字段要求存于现有 `capability_target`，不扩展数据库表。

[Control gateway](../../crates/control/src/api/brain_runs/v4/gateway.rs) 解析实际输入，缺失引用或字段以 422 回执结束该次派发，原因与操作终态一同持久化并重新唤醒大脑；临时不可用继续重试。[Worker 输出适配](../../crates/worker/src/brain/v4/output.rs) 向大脑传递完整结构化结果，`summary` 不能覆盖其他字段。叶子能力的成功决策证据限 16 KiB；缺失必填输出或超限转为 Error，完整结果保留在子执行，失败上下文明确标记省略部分。Done 仅表示能力执行结束，里程碑是否达标由大脑判断。

Brain 管理的执行各有工作区，见 [工作区解析](../../crates/worker/src/brain/workdir.rs)；跨执行的代码版本和产物需要显式传递。DAG 从 `layered_inputs` 取得命名输入及二进制参数。原生闭环与版本传递见 [brain_closed_loop.rs](../../crates/worker/tests/brain_closed_loop.rs)，输出和拒绝回执见 [brain_contracts.rs](../../crates/worker/tests/brain_contracts.rs)、[brain_dispatch_failures.rs](../../crates/worker/tests/brain_dispatch_failures.rs)。

- `crates/core/src/brain/layered/`：计划、运行、操作与决策协议。
- `crates/core/src/brain/capability.rs`：能力描述及输入引用。
- `crates/brain/src/layered/`：图校验、分层、上下文、决策、终态、重试与命令纯函数。
- `crates/brain/src/runtime.rs`：能力库持久化及向量检索。
- `crates/control/src/api/brain_runs/plan_capabilities.rs`：保存计划能力化及嵌套引用校验。
- `crates/control/src/api/brain_runs/v4/`：准入、派发、事件转交及索引视图。
- `crates/worker/src/brain/v4/`：节点投影、恢复、模型调度与确认。
- `crates/brain/tests/milestone.rs` 与 `crates/brain/tests/milestone/`：图约束、层屏障、回退与终态；`crates/worker/tests/brain_scheduler_v4.rs`：节点调度；`crates/worker/tests/brain_nested.rs`：嵌套计划链路。
- [CI 入口](../../scripts/ci/brain.py) 分阶段准备同版本原生镜像，并验证项目恢复、里程碑、调度、Server 重启和真实浏览器；编译与原生运行分开处理权限，临时目录避开 runner 私有父目录，失败保留原始错误和页面证据。运行条件见 [验收说明](../../scripts/acceptance/brain/README.md)。

新计划和运行入口要求 schema 7；历史 schema 4/5/6 只读；`v4/` 是现存实现目录名，历史读取逻辑保留在代码中。历史数据清理使用 `scripts/maintenance/brain_cleanup/` 的审阅清单、行摘要校验、备份与重复复核；清理范围必须同时覆盖运行数据、Server 索引及 Host 休眠索引，避免节点同步恢复已删除的 ID。清理不在存储初始化中自动执行。

[调度规则](../../rules/06-brain-scheduling-contract.md) · [运行协议](../../docs/brain-orchestration.md) · [工作台](../../features/brain/index.md)
