# 基线失败四项根因闭环：测试侧修复，零生产改动

基线失败的四个根因全部定位并修复。全部改动限于测试与测试基建，
不触 rules/04/06/07 任何不变量；生产 admission/lifecycle 锁语义、
控制面产物门文案均保持现状。

## 根因与修复

| # | 失败项 | 根因 | 修复 |
|---|--------|------|------|
| 1 | `web agent_model_toctou::agent_switch_wins_mutex…`（409 vs 200） | 测试侧时序假设不成立：agent 切换在拿锁前有一次真实异步 store 往返（`api.rs` reject_node_session→get_session），drain 侧零往返直达 lifecycle 锁；一次 `yield_now()` 只保证任务被 poll 过、不保证已排入锁队列，drain 稳定抢锁先赢 → 409 是设计内行为 | 重排机制不改语义：删 yield 排序，改为 spawn agent → drop(lifecycle) → 轮询临界区内可观测写入（`overrides.agent`，仅在持锁时发生）→ 再 spawn drain；胜负来自互斥排除而非时序。与 drain-wins 兄弟测试保持双向对称覆盖 |
| 2 | `web dag_e2e_flow` ×2（fixture panic） | 环境双重缺口：(a) fixture 默认路径按 `current_exe().parent().parent()` 推导，而测试二进制实际从 `target/debug/build/<pkg>/<hash>/out/` 执行，默认路径永远落空（CI 靠 `DAG_TEST_ROOTFS` 注入才成立）；(b) 本机镜像 build_info 锚定旧提交，freshness 检查按 rules/04 镜像语义正确拒绝 | 重建镜像锚定当前 build_info（`scripts/prepare-dag-rootfs.sh`，宿主机补装 `rg`）；fixture 默认解析改为沿祖先有限层数（8 层）查找 `target/<profile>/dag-rootfs`，找不到回退原默认路径保持 fail-fast；行内提示提升为 `ROOTFS_HINT` 单源常量。改共享 fixture 本体（`dag-runtime/tests/support/container.rs`），全部跨 crate 消费者（web/worker/project/dag-runtime）同时受益 |
| 3 | `control executions_paging::artifact_streams…` | 陈旧断言：2026-10-13 起（changelog `execution-context-and-artifacts.md`，rules/06:53-55 契约）产物访问放宽到 Project/Brain/Operator + 托管 Agent/Team/Todos，控制面文案改为 `execution does not expose artifacts`（`api/streaming/artifact.rs`）；当次定向验证过滤词 `"artifacts"` 不匹配 `"artifact_streams"` 导致漏改 | 断言对齐现行契约 `execution does not expose artifacts`；测试名 `…_for_dag_runs_only` → `…_and_kind_gate`（端点已服务多种执行类型；managed 门控由 `executions_artifacts/managed.rs` 精确覆盖） |
| 4 | 全量并行 e2e 大面积失败（基线 ~65，隔离全过） | 测试基建资源竞争：~150+ 测试各自全栈启动，Ncpu 并行下紧上限断言与紧 deadline 在 CPU 饥饿下翻转 | 本次不修（见下「后续」）；终验以 `--test-threads=2` 区分基建竞争与真实回归 |

## 测试覆盖（改动 → 定向测试）

| 改动 | 定向测试 | 结果 |
|------|----------|------|
| executions_paging 断言/改名 | `cargo test -p opencoder-control --test e2e executions_paging` | 6/6 过 |
| agent_model_toctou 重排 | `cargo test -p opencoder-web --test agent_model_toctou` | 连跑 5 次全绿（2/2 ×5） |
| container.rs 默认路径 | `cargo test -p opencoder-web --test dag_e2e_flow`（不设 `DAG_TEST_ROOTFS`） | 2/2 过 |
| clippy | `cargo clippy --tests -p opencoder-control -p opencoder-web -p opencoder-dag-runtime` | 本次改动零警告（`dag-runtime/src/runtime/dynamic.rs` 存在 HEAD 既有 `needless_range_loop` 警告，生产文件，不在本次范围） |
| 终验 | `cargo test -p opencoder-control --test e2e -- --test-threads=2` | 262/262 过（154s）；基线 ~65 项并行失败确认为测试基建竞争，归档至阶段 2 |

## 明确不做（红线）

- 不动生产 admission/lifecycle 锁语义（#1 的 409 是设计内行为）；
- 不给 dag fixture 加 skip/ignore/env 门（fail-fast 是 rules/04 的镜像契约）；
- 不把控制面产物门文案改回旧串（产物放宽是被契约接受的决策）；
- 零生产代码改动。

## 后续（另立任务分批）

- 阶段 2 并行稳定性专项：rules/03 增补并行执行策略、重度目标 `--test-threads=2` 脚本、deadline 缩放助手、Harness 复用 `reqwest::Client`、共享目录 override 收口、端口 TOCTOU 修复。
