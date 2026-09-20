Commit: 5c7fc73fcadde684cbf5e29811b8e8f2352690e1

# 大脑调度工作台

平台同时支持两种脑运行。v2 以 input、实例、output、路由四类概念编辑、发布和运行不可变计划；v3 以工程输入和能力目录按轮次调度，等待子执行终态事件后再次判断。两者都覆盖 Agent/DAG/Team/TODO/Operator，v2 保留图内并行、因果汇合与修复循环。

发起运行时输入由一段语义化描述（目标和交付物）与工程描述组成：工程描述在 Web 上以一层 KV 对的 itemlist 组织，键为计划声明的输入端口名，值支持 JSON 字面量；下游输入承接连线指定的输出内容和产物。每个实例来自已注册能力，发布时固定定义；运行期间不能增加节点或更改路线。

工作台显示各轮输出、活跃分支、等待原因，以及路由读取了哪些输出、为何选择下一步。完成与验证分开展示，缺少依据时为未知。无匹配路线、非法选择、缺少输出或达到访问上限会明确阻塞；只有满足声明出口交付条件且所有已激活分支收敛才完成。

历史计划与证据可只读查询，旧格式写入和执行要求迁移。存在旧非终态运行时阻止升级，待所属旧运行正常收敛后再切换协议。

### v3 运行规则

创建 v3 运行时必须声明 `schema_version: 3`。每轮只能派发能力目录中有真实 target、definition、版本及输入/输出描述的能力；输入只能绑定根请求命名输入、成功执行的 `execution_id` 输出路径或已有产物引用。当前轮次全部成功终态才进入下一次判断；任一 Error/Cancelled 终态立即失败并取消兄弟执行。`Complete` 必须引用成功终态执行，非法决策、无证据完成和超过轮次上限会阻塞。

运行快照只展示脑状态、轮次和 operation 索引。消息、DAG 步骤、Team 对话、TODO 项和产物正文通过执行 ID 在所属节点查询；重复、乱序或迟到终态事件不会重复推进或改写已终态运行。CLI 和 Web 提供事件页、轮次查询及 pause/resume/cancel。

v3 Web 工作台先展示目标、阶段、当前轮次和结果的摘要画布，再按轮次展示关联能力索引。当前轮默认展开、历史轮次折叠；点击能力的 execution ID 打开只读执行抽屉，继续复用 Agent、Team、DAG、TODO、Operator 的执行组件，因此 DAG 步骤日志和其他执行明细仍从所属节点实时读取。

### 浏览器草稿缓存

计划编辑器草稿按 `oc:brain:plan-draft:<owner>:new|<id>@<version>` 键存于 localStorage，读取按当前协议校验，损坏或旧协议残留 fail-closed 不覆盖原文。旧协议（v1 `plan.steps`）残留不再死锁编辑器：错误指引文案 + 「丢弃缓存并重新开始」逃生口——原文先备份到 `<key>:legacy-v1` 单槽再重置为干净 v2 草稿（新建为空计划、编辑为服务端版本快照）。已随 rel-5c7fc73f（0.1.0，2026-09-20）发布上线并完成验收：工作台 v2 通道（`/api/brain/plan-defs` validate→save）线上冒烟通过，观察期无 failure；v1 `/api/brain/plans` 的 409 为 schema_version:2 设计门禁。详见 [changelog](../changelog/2026-09-20/brain-draft-legacy-cache-recovery.md)。

## 相关
- [agents/brain](../../agents/brain/index.md) — 状态机与回归
- [agents/control](../../agents/control/index.md) — 派发 API
- [运行协议](../../docs/brain-orchestration.md)
- [固定图 v2 与验收映射](../changelog/2026-09-16/brain-fixed-graph-v2.md)
- [草稿旧协议缓存恢复](../changelog/2026-09-20/brain-draft-legacy-cache-recovery.md)
- [事件驱动调度 v3](../changelog/2026-09-18/brain-scheduler-v3.md)
