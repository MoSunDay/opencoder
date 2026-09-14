Commit: 2686d40a267436adb555041fc8154fc9bb454574 (working-tree)

# brain 模块

纯本体校验、有限调度状态机和一次动态规划；保留描述型能力、向量检索与决策树路由接口。`ActionFlow` 在同一计划内按条件边创建持久访问轮次，`flow_current` 指向当前轮次；`execution::flow` 在回执后推进条件、回退、交付物校验和取消边界。

## 类型与边界

- 共享 DTO 在 [core/brain](../../crates/core/src/brain/)。PlanVersion 是不可变计划快照，StepTemplate 定义类型端口、输入绑定、控制依赖、条件、有限 foreach、动作和资源约束。
- BrainRun 固定意图和计划，保存展开实例、输入请求、交付物、Prepared 动作、来源游标及 activation/control_epoch。版本成熟度与置信依据独立。
- 调度域函数显式接收状态、事件和时间，不执行网络或文件副作用。运行中不替换计划；资源占用与派发由控制面处理。

## 关键路径

- [ontology/](../../crates/brain/src/ontology/) — JSON schema 子集、绑定来源、类型/语义、控制循环及批量汇合校验。
- [execution/](../../crates/brain/src/execution/) — initialize/advance、输入解析、稳定实例 ID、条件展开、通知去重、动作账本、暂停/取消（取消会在无活动子动作时收敛为终态）和交付验证。
- [activation.rs](../../crates/brain/src/activation.rs) — 固定模式返回全部就绪动作；动态模式调用 ChatStream 一次，解析完整计划后校验，不带运行期工具循环。
- `src/{domain,runtime,plan,planning}.rs` — 描述型能力、向量检索和兼容决策树；旧 `brain_plans.tree_json` 与新本体计划版本分开存储。

## 主流程

1. control 固定能力、定义、资源摘要和配置快照，以幂等 ID 受理 Brain 根运行。
2. worker 使用 TODO 状态提交保存根状态与因果事件，短 runc 激活挂载 CLI，读取独立 context。
3. 动态模式生成的完整候选计划经 control 校验、追加版本后自动继续；固定模式读取明确版本。
4. 纯调度推进就绪实例并先保存 Prepared 动作。control 复核激活代际、跨运行资源占用和目标节点资源摘要，再向原生执行器派发。
5. 来源节点持久重放状态/输出通知；根提交后才确认游标。明确结束回执释放资源，后继立即就绪，无整批屏障。
6. 等待子执行或输入时释放根槽位；同盘重启恢复原状态和动作。暂停阻止新派发，取消等待在途执行明确结束。

## 跨模块接口

- [control](../control/index.md) — `api/brain_runs/`：计划/能力/运行 API、版本发布、派发、资源占用与回执。
- [worker](../worker/index.md) — `brain/`：根持久化、短激活、outbox、恢复和四类结果适配。
- [store](../store/index.md) — 追加版本、稳定指针、全局资源占用、原子 TODO 事件水位。
- [node](../node/index.md) — Brain RPC 与可重放上行消息；[ctl](../ctl/index.md) — 有限本地激活入口。
- [web](../web/index.md) — 工作台投影与原生过程组件；[产品契约](../../features/brain/index.md)、[协议示例](../../docs/brain-orchestration.md)。

## 验证入口

- [ontology.rs](../../crates/brain/tests/ontology.rs)、[execution.rs](../../crates/brain/tests/execution.rs)：类型、非屏障并发、输入、控制、批量和输出契约。
- [brain_versions.rs](../../crates/store/tests/brain_versions.rs)：不可变版本、稳定指针和跨运行读写互斥。
- [brain_ontology.rs](../../crates/worker/tests/brain_ontology.rs)、[brain_outputs.rs](../../crates/worker/tests/brain_outputs.rs)、[brain_recovery.rs](../../crates/worker/tests/brain_recovery.rs)：真实节点通道、输出产物和重放恢复。
- [container.rs](../../crates/worker/src/brain/container.rs)：需要 runc 权限、显式执行的挂载 CLI 冒烟；[UI 测试](../../crates/web/spa/src/brain/workbench/tests/)。
