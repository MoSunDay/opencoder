# Brain v3 创建与验收收尾

Web 新建大脑运行此前仍提交 v2 的 mode/plan/references，并读取旧回执 id，无法与已启用的 v3 接口协作。现改为 schema_version: 3、命名 inputs、capability_ids 和 max_rounds；按匹配的 run_id 接受回执，网络重试保持同一运行 ID。无工程参数时以 request 输入传递目标；重复参数名明确报错。

历史计划与 v2 运行在工作台只读。轮次推进时自动展开新轮次，历史轮次可手动查看；执行抽屉按 execution_id 隔离组件状态，仍复用原能力执行面板。

节点恢复在取得生命周期锁后重新读取运行状态和上下文，避免调度上下文已经入队时使用旧 Idle 记录重复入队，进而把节点标记为持久化故障。

Brain 管理的 Agent 与历史 Operator 通过内部请求沿用入队时的模型、提供方及 AP 设置。新 Operator 使用执行专属 HOME 下的冻结配置和独立 workspace；快照以 0600 权限原子持久化，记录隔离版本，缺失或损坏时明确失败。提示、压缩、交接及恢复保持同一配置规则。

节点读取 outbox 不再无条件发送状态变化通知，消除“报告 → 通知 → 再报告”的循环；仅持久状态提交或终态首次收敛发送通知。Agent 恢复执行仍使用其隔离目录。

子执行提示明确能力 ID、类型、target 和输入输出契约，并将根目标限制为上下文；Team 只判断自身产出完成，不再重复根任务调度或等待兄弟执行 ID。

隔离 CLI、Server 与 Worker 共用模型推理参数的默认值合并逻辑，沿用已配置的 reasoning_effort 并保留请求级覆盖；请求级 HTTP 测试核验参数真正发往模型提供方。无效模型 JSON 仍明确阻塞，错误信息标明协议要求。

Control 合并处理中的重复 outbox 投递，避免查询触发的重复上报耗尽事件处理槽位。唤醒确认只覆盖来源事件或本次实际接收的 context generation，节点确认游标保持单调，防止旧确认吞掉新轮次或倒退。

SPA 保留锁定 Terser，并将 CommonJS strictRequires 固定为 true。此前 dayjs 的条件引用检测会竞态地产生直接初始化或函数包装两种产物；单换压缩器不能解决。漂移门禁改为单次字节比对，不再通过重试接受不稳定产物。

| 场景 | 验证入口 |
| --- | --- |
| v3 提交、回执、幂等重试、工程 JSON 参数 | SPA workbench launch/model 测试 |
| 首轮/次轮自动展开、历史 v2 只读、执行抽屉 | SPA workbench v3Run 测试 |
| 页面创建 → 真实 Control/Worker → 执行索引 | Worker brain_browser 显式 Chromium 测试 |
| DAG → Agent/Operator/Team/TODO、输出引用、屏障、暂停/恢复、Step 日志 | scripts/acceptance/brain/fixture.js（模型传输固定，执行器与 HTTP 真实） |
| 同一五类能力场景的真实模型验收 | scripts/acceptance/brain/isolated-real.js（隔离进程）与 live.js（发布后的配置文件和证据目录参数） |
| 可重复构建 | scripts/check-spa-drift.sh |
| 并行屏障、重复/乱序/旧轮次事件、轮次上限、引用校验 | crates/brain/tests/scheduler/barriers.rs |
| 终态事件唯一键、事务回滚、分页与重启恢复 | crates/store/tests/brain_scheduler_v3.rs |
| 报告读取不自触发、旧确认不吞新轮次、重复投递合并 | worker brain_report 与 control brain_delivery 测试 |
| 恢复与上下文入队竞态 | crates/worker/src/brain/wake/tests.rs |
| 单个子执行失败后取消在途兄弟执行且不再决策 | crates/worker/tests/scheduler/failure.rs |

脚本保留验收执行和索引回执，不删除数据库、不记录凭证明文。生产发布状态以独立发布回执为准。

本次同时整合动态 DAG 实例和 Operator 隔离。Brain v3 根运行与子执行通过 brain_scheduler_v3 排除未升级节点，普通任务保持原有路由。动态 DAG 通过现有 capability_probe 协商 dag_dynamic_v1，派发前排除旧节点，实例与产物接口避免向旧节点发送未知操作。动态模板收到能力的调度提示，根输入引用在直接派发与 Brain 派发时保持相同形状。

补充验收：brain/fixture.js 从 Web 创建五类能力任务；brain/dynamic.js 验证 Brain 执行抽屉中的动态实例、实时日志、输入绑定和整组屏障；dag_dynamic.js 验证原 DAG 页面相同面板；brain/repair-loop.js 固化测试、修复、复测的三轮闭环。模型脚本与真实模型验收分别记录，不混用结果。

创建响应丢失后的幂等重试现在先读取节点持久化记录，核对原请求并返回索引，不再排在其他任务的冷资源复制后面。新接收仍取得全局创建锁，并在锁内再次检查同一 ID，保留参数冲突拒绝和重复创建保护。锁竞争测试覆盖已接收任务、参数冲突、节点不匹配，以及新任务仍须等待接收锁。

发布验收分别记录冷资源准备耗时与切换期间的请求指标。准备阶段重用相同执行 ID 恢复确认，计量阶段仍禁止重试；异常退出等待本次 WASI 的持久接收记录后释放专属门闩，不提前创建执行目录。已独立验证首次激活时，可通过 current-roundtrip 模式继续核验持有真实任务的信号回滚、再次发布和完整观察窗口。
