Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# 当前 Server 的大脑调度验收

新增 20 项可分批运行的验收，通过公共 Server、真实模型和原生 DAG 验证调度规则。主场景让普通算术测试通过、负数边界测试实际失败，检查大脑根据完整结果修改源码，再验证同一新版本。预期路径只存在于断言中，模型收到业务目标和执行证据。

每批核对实际 Server、Runtime 可执行文件的提交与摘要，保存独立 HTTP、执行、决策和代码版本证据。汇总保留旧失败及最新结果，前提不足明确标为 `NOT_RUN`。重试使用相同运行 ID；清理只处理本批根运行，并等待子执行终态。故障用例先等其他任务结束，强杀前只读检查本地 libSQL 运行名额，恢复路径不足时不执行故障。

这是验收工具变更，未修改产品调度逻辑、数据库结构或配置项。故障测试需单独获得当前机器演练授权；CI 运行脚本自身的本地测试。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 事件唤醒、整层屏障、结果判断、跨层整改与版本传递 | `closed-loop` | [closed_loop.py](../../../scripts/acceptance/brain/scheduling/cases/closed_loop.py) |
| 字段、规模、版本、嵌套深度与节点要求 | `admission`、`definitions`、`context-capacity` | [admission.py](../../../scripts/acceptance/brain/scheduling/cases/admission.py)、[definitions.py](../../../scripts/acceptance/brain/scheduling/cases/definitions.py) |
| 暂停、输入、恢复、取消与轮次预算 | `controls`、`round-budget` | [control.py](../../../scripts/acceptance/brain/scheduling/cases/control.py) |
| 六种能力真实执行、固定版本嵌套计划 | `six-types` | [execution_types.py](../../../scripts/acceptance/brain/scheduling/cases/execution_types.py) |
| 重复和迟到回执、过期 generation、终态只读 | `receipt-replay` | [replay.py](../../../scripts/acceptance/brain/scheduling/cases/replay.py) |
| 同层失败屏障、超限输出、补条件恢复、同层整改 | `failure-barrier`、`oversize-output`、`missing-prerequisite`、`same-layer-repair` | [negative.py](../../../scripts/acceptance/brain/scheduling/cases/negative.py)、[definitions.py](../../../scripts/acceptance/brain/scheduling/cases/definitions.py) |
| 必填字段、有效空值、产物、冻结约定、请求重放 | `contracts`、`frozen-contract` | [contracts.py](../../../scripts/acceptance/brain/scheduling/cases/contracts.py) |
| 运行中向 Agent、Operator、Team 引导 | `live-steering` | [real.py](../../../scripts/acceptance/brain/scheduling/guidance/real.py) |
| 临时拒绝重试、Server 与 Worker 恢复 | `dispatch-retry`、`process-recovery` | [recovery.py](../../../scripts/acceptance/brain/scheduling/cases/recovery.py) |
| 三次纠错与五分钟截止时间 | `correction-budget`、`correction-deadline` | [correction.py](../../../scripts/acceptance/brain/scheduling/cases/correction.py) |
| 满载排队、固定节点与不重复执行 | `capacity-queue` | [queue.py](../../../scripts/acceptance/brain/scheduling/placement/queue.py) |
| 实际执行详情、日志与四种屏宽 | `browser/verify.js` | [verify.js](../../../scripts/acceptance/brain/scheduling/browser/verify.js) |
| 真实数据断言、不可覆盖证据、同 ID 重试、归属与恢复保护 | 23 项脚本单测 | [tests](../../../scripts/acceptance/brain/scheduling/tests) |

## 验证结果与剩余问题

- `cargo test --workspace`：5,686 passed / 0 failed / 7 既有 ignored；原始输出位于 `/var/tmp/opencoder-brain-scheduling-20261007/workspace-tests-r4.log`，摘要见同目录 `rust-regression-summary.json`。
- `cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace`、Rust 格式检查通过。
- SPA 全量 134 文件、980 项通过；验收脚本 23 项、发布工具 66 项、CI runner 7 项、浏览器失败处理 2 项通过。真实闭环的八个执行详情与四种屏宽页面验证通过。
- 当前部署 `4bb3a745544f3b3f9898447088a919da502de6e5` 的真实验收结果为 13 PASS / 2 FAIL / 5 NOT_RUN。汇总和历史证据位于 `/var/tmp/opencoder-brain-scheduling-20261007/acceptance-final.json`；这不代表全部调度规则已通过。
- 主闭环实际路径为 `1→2→3→1→2→3`，共两轮、八个独立原生执行；进程成功但业务失败的结果触发返工，修复后的源码摘要与复验版本一致。
- `missing-prerequisite` 失败：人工补充表达式后，模型提出第 0 层派发，三次纠错后再次阻塞。上下文中的初始层位置为 0，错误反馈只给出 `unknown target layer`；后续修复应明确可派发层编号及 `layer_id` 对应关系，并让校验反馈给出有效范围，再验证恢复后的实际输入和输出。禁止通过放宽层约束或增加纠错次数掩盖问题。
- `context-capacity` 失败：当前 Runtime 未声明 `brain_context_budget_v1`。需完成对应产品改动的验证与发布后，重新验证超限准入和累计人工输入限制。
- 五项故障用例等待 30 分钟后仍因其他任务运行与本地存储不足未执行。尚需在空闲且资源就绪的窗口复验；活动模型调用期间的 Worker 强杀，还需已验证的 Host 名额恢复路径。不能通过删除 libSQL 记录制造恢复成功。
- 最终只读检查确认 30 个测试根运行均已结束或未创建，子执行均已终态。之后公共就绪检查出现 503，本地节点上报资源挂载 I/O 错误；版本一致不代表环境健康。本次没有实施进程故障或发布，环境问题另存于 `closure-audit/baseline.json`、`final-nodes.json`。

[验收入口](../../../scripts/acceptance/brain/scheduling/README.md) · [调度逻辑索引](../../../agents/brain/index.md) · [调度规则](../../../rules/06-brain-scheduling-contract.md)
