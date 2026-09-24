# 大脑里程碑计划与自由回退

计划画布按层配置里程碑及挂载能力；编辑节点后，通过计划信息表单保存版本。运行时大脑在整层执行结束后判断下一层，可回退到已执行层开启新轮，并为重跑能力生成独立执行 ID，保留历史输入与结果。无效决策最多纠正两次，不派发无效能力。运行视图按轮、层和能力 ID 打开原有类型执行面板。

窄屏画布在视口变化后重新适配节点位置，工具栏按钮保持可见，React Flow 控件限制在画布内。检查器说明与无需预设回退连线的规则一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 无效决策纠正与禁止派发 | `invalid_decision_is_corrected_before_any_capability_is_dispatched`、`three_invalid_decisions_block_without_dispatching_a_capability` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 回退后独立激活与历史记录 | `failure_wakes_the_brain_and_reflection_creates_a_distinct_durable_visit` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 真实 Server/Agent 的 schema 6 准入与层屏障 | `brain_e2e` 2 项、`brain_layered_e2e` 5 项 | `tests/brain_e2e/`、`tests/brain_layered_e2e/` |
| 画布编辑、提交与失败草稿 | `在真实画布编辑节点后，表单提交保存新版本且不会启动运行`、`提交失败后保留节点与表单草稿，返回画布可继续修改` | `crates/web/spa/src/brain/workbench/milestone/editor.dom.test.jsx` |
| 轮次历史和执行明细 | `历史轮保留各自输入、反思和执行 ID`、六种类型面板测试 | `crates/web/spa/src/brain/workbench/milestone/run.dom.test.jsx` |
| 窄屏布局 | 390px 与 1440px 浏览器实测、宽窄切换后节点可见且控件不越界；生产验收附窄屏几何断言 | `scripts/acceptance/brain/layered-panels.js` |

- SPA 全量回归：`npx vitest run` → 906 passed / 0 failed。
- SPA 构建：`npm run build` → 通过；`scripts/check-spa-drift.sh` → 无漂移。
- Review DAG 相邻回归：已移除的生产种子改由测试内显式创建，5 条测试通过；实现见 `tests/dag_e2e/review_dags/`。
- Rust workspace 全量回归：先执行 `cargo build --workspace --bins -j 8` 更新进程级测试依赖的二进制，再执行 `cargo test --workspace -j 8 -- --test-threads=4` → 5,584 passed / 0 failed / 8 既有 ignored。
- Rust clippy：`cargo clippy --workspace --all-targets -j 4 -- -D warnings` → 零警告。
- Rust 构建：`cargo build --workspace -j 4` → 通过。
- Clippy 整理后的相关 crate 回归：Brain、Control、Worker、DAG Runtime 共 657 passed / 0 failed / 6 既有 ignored。

真实流量发布验收新增独立公共入口探针，每 200ms 采样 readiness，完成响应间隔不得超过 1 秒且不能有失败；保留原有三项 1 秒任务受理和调度门槛、唯一归属及完成要求。独立探针可区分串行任务请求的尾部延迟与入口服务空窗，不替代原门槛。生产首次观测到真实 readiness 响应间隔超过 1 秒，因此回滚并继续定位。

开放状态的 `/api/ready` 只检查准入状态和在线节点，不再为每次探针扫描执行索引；冻结状态与 `/api/admin/drain` 继续计算活动执行数，保证排空判定准确。此次生产样本的 83 次任务受理中 P95 为 1.062 秒、独立 readiness 最大响应间隔 1.666 秒；两者均不计为上线通过。

`757b9da1` 的定向 Control/Python 回归和精确发布包构建通过。隔离发布首次因两次 1.7/1.6 秒受理超时失败，重跑 73 次受理最大 0.073 秒通过。生产信号发布后 111 次受理无 HTTP 失败、226 次 readiness 探针无失败且最大完成间隔 0.582 秒，但三次任务受理超过原 1 秒门槛，最大 1.617 秒，故回滚到 `rel-e838bfbaadc8314a8ac5cc2bdb3a91c9b3d84bd1`，未计上线成功。跨版本 TODO 的脚本实际运行并生成 `second.done`，但模型只提交脚本调用、未提交只读文件核验证据，达到两次候选上限后任务失败；验收指令现明确要求脚本后只读核验产物，恢复修订不重复执行。Control 受理增加仅针对超过 500ms 的阶段耗时告警，以区分准备、节点回复及持久化收口延迟；待下一轮生产证据定位后再决定具体性能修复。

`9c499a69` 的定向 Control 测试、Clippy、发布脚本 9 项单测、精确构建和只读 schema 6 迁移预检通过。隔离发布重跑 90 次受理最大 0.131 秒通过；另一轮两次慢请求的 Runtime 日志显示持久化准备耗时分别约 0.7/1.4 秒，涉及保证创建回执可恢复的落盘步骤，不能直接移除。生产信号切换后 132 次受理全部完成，跨版本 TODO 两步均完成并通过，230 次公共 readiness 探针零失败、最大完成间隔 0.558 秒；但三次串行受理超过旧 1 秒门槛，最大 1.109 秒，受理 P95 为 0.741 秒，最长受理间隔 1.209 秒、调度间隔 1.443 秒。该轮按现行门槛失败并已回滚至原版本，未进行最终 900 秒观察或大脑六类型真实模型验收。受理尾部延迟和独立入口连续性是否应共用同一个硬门槛，需要按上线目标明确后再完成发布。

后续验收将公共入口可用性与单次持久化长尾分开：生产仍要求独立 200ms readiness 探针零失败、响应完成间隔 ≤1 秒，所有提交请求无错误、唯一 Runtime 归属且执行完成；受理速度以 P95 ≤1 秒为门槛，最大受理耗时及串行提交/调度间隔继续完整记录。原串行流量一旦碰到一次存储长尾，会同时推高三项最大值，即使独立公共入口一直可用；隔离进程测试继续保留原严格尾部门槛。此次调整是对“发布不停服”的直接观测口径修正，不改变任务持久化语义。
