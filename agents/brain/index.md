Commit: 0391933f

# brain 模块

能力目录与 schema_version 7 里程碑调度。`layers` 是有序的里程碑容器，必填名称、任务、目标和达成标准；`nodes` 通过 `layer_id` 归属一层，每个节点仅绑定一个泛化能力，节点标题与任务由所选能力生成。Brain 在整层并行执行终态后评估当前层：达标后前进到紧邻下一层，需整改时可选择本层或任一已执行层；旧 `transitions` 可读取，但不参与模型决策。新计划在保存和运行准入时由纯函数补齐内部前进与回退路径，兼容现有运行图校验与不可变重试，用户无需配置这些路径。回退开启新轮次，每次激活保留独立执行 ID；能力负责具体任务，大脑负责绑定输入、判断状态与选择下一层。所有能力使用相同的决策校验，不按能力名称增加专用限制。

运行创建、层屏障和人工输入是决策唤醒事件。人工输入持久化后令当前决策失效并重新组装上下文；层屏障未满足时只允许 `guide`，不能派发下一层。`guide` 可向当前运行中的 Agent/Operator 投递 steer，或让 Team 在下一次成员发问时应用引导，投递事件按序确认并可重放。生产模型激活经 OpenCoder session agent loop，每次唤醒只产生一个有限决策。

`run.reflection` 保留此前返工的原因；它不是下一次决策必须复制的字段。返工后正常前进仍要求决策的 `reflection: null`，推进理由放在 `reason` 和当前层的 `assessments`。提示词和拒绝反馈明确这一区别；校验继续拒绝携带反思的前向派发。

调度范围是计划已绑定的能力，每次派发覆盖目标层全部节点；运行中不检索能力库来增加节点或替换能力。能力描述、输入输出描述及必填字段随能力定义冻结，进入调度与执行提示词。[字段校验](../../crates/brain/src/contracts.rs) 负责字段名、输入值和输出证据的纯函数校验；字段要求存于现有 `capability_target`，不扩展数据库表。

[容量预算](../../crates/brain/src/layered/budget.rs) 按完整计划、冻结能力、各节点结果上限和输出预留计算准入容量，包含 JSON 转义与嵌套回执。Control 携带真实请求探测支持 `brain_context_budget_v1` 的节点；[节点预算](../../crates/worker/src/brain/v4/budget.rs) 按有效模型配置再次检查并冻结预算。人工输入按累计内容检查，实际决策上下文在模型调用前复核；超限不裁剪成功证据，准入返回 413，存量超限运行保留证据并明确阻塞。上限及部署顺序见调度规则。

[Control gateway](../../crates/control/src/api/brain_runs/v4/gateway.rs) 解析实际输入，缺失引用或字段以 422 回执结束该次派发，原因与操作终态一同持久化并重新唤醒大脑；临时不可用继续重试。[Worker 输出适配](../../crates/worker/src/brain/v4/output.rs) 向大脑传递完整结构化结果，`summary` 不能覆盖其他字段。叶子能力的成功决策证据限 16 KiB；缺失必填输出或超限转为 Error，完整结果保留在子执行，失败上下文明确标记省略部分。Done 仅表示能力执行结束，里程碑是否达标由大脑判断。

一次根 run 独立保存 context 和执行索引，各能力用自己的工作区并通过输入、输出与文件引用交接。[引用校验](../../crates/brain/src/layered/validate.rs) 对普通及共享目录计划统一要求本次 run 中该逻辑节点最新 activation 的终态，失败证据也可绑定。执行绑定默认读取 output，`source: artifacts` 读取文件索引；两者都使用 JSON Pointer。Control 从所属节点读取完整结果及引用并放入 `summaries`、`execution_artifacts`，Worker 校验引用的执行归属并计入容量。

叶子输出与文件引用合计限 16 KiB；[Worker 索引](../../crates/worker/src/brain/output/index.rs) 将较大索引保存为带内容摘要的清单引用，嵌套 Brain 通过清单交接子文件。清单只压缩索引，保留结构化成功证据；无文件时为空数组，缺失、摘要不符、路径逃逸或软链接会形成执行错误。完整链路见 [brain_io](../../crates/worker/tests/brain_io/main.rs)。

Brain 默认各执行独立工作区；计划 `shared_workspace: true` 时仅允许 Operator，在根节点共享任务目录并按 activation 留存证据，`adjacent_only: true` 则强制只退回相邻上一层或本层。路径与归属由 [共享工作区](../../crates/worker/src/brain/workspace/mod.rs) 在受理时冻结，见 [工作区解析](../../crates/worker/src/brain/workdir.rs)；跨执行的代码版本和产物需要显式传递。DAG 从 `layered_inputs` 取得完整命名输入及二进制参数，恢复拒绝保存输入漂移。新根 Brain 和托管子执行要求 `execution_io_v1`，先升级 Worker 再启用 Server。原生闭环与版本传递见 [brain_closed_loop.rs](../../crates/worker/tests/brain_closed_loop.rs)，输出和拒绝回执见 [brain_contracts.rs](../../crates/worker/tests/brain_contracts.rs)、[brain_dispatch_failures.rs](../../crates/worker/tests/brain_dispatch_failures.rs)。

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

能力更新可携带内容版本：Runtime 在 embedding 前检查一次，Store 在提交事务内再次检查，覆盖计算期间的并发编辑。旧调用兼容，embedding 失败不修改已有文字、样例和向量。见 [runtime](../../crates/brain/src/runtime.rs)。

能力目录核对显式 Codex profile 引用：先检查中央登记，再通过在线节点只读 `execution_profiles` 操作取得 Agent 与 Operator 各自的配置名称。确认缺失时保留完整目录记录并标记不可用，选择目录过滤不可用项；旧节点不支持查询或暂时不可达时保留原兼容行为。查询不返回配置值，不修改已受理执行的冻结配置。见 [可用性检查](../../crates/control/src/api/brain_runs/catalog/availability.rs)。
