# DAG 节点执行运行时：逻辑评审缺陷修复（并发双发 / busy guard / 超时清理 / 取消落盘 / 终态原子性）

日期：2026-09-05 ｜ 提交：(working tree)

## 动机

对 DAG 节点执行运行时（commit af71944 工作树）的逻辑层评审发现 5 项缺陷（1 严重 + 4 中）：全量回归绿但测试盲区（无并发步骤交错完成用例）放过了核心调度缺陷。本次修复全部 5 项并补齐盲区回归用例。

## 变更

### #1 并发步骤双发（严重）

- `crates/dag-runtime/src/runtime.rs`：spawn 循环此前只看 `ready_steps()`（仅排除已写入 `states` 的完成步骤），每完成一步就重派仍在运行的兄弟步骤——重复 LLM 会话/事件/工件覆盖、实际并发可超 `MAX_CONCURRENT_STEPS=4`、runc 容器 id 撞车。新增 `inflight_names: BTreeSet<String>` 在飞集合，spawn 前插入、完成（主循环与取消 drain 两路）移除，spawn 循环过滤在飞步骤。

### #2 claim busy guard 漏 cancelling（中）

- `crates/store/src/libsql_store/dag.rs::claim_next`：单活跃检查从 `status = 'running'` 补为 `status IN ('running','cancelling')`（与移植源 `node_tasks.rs::claim_next` 一致），取消收尾窗口内节点不可再 claim，恢复单活跃不变量。

### #3 双重 timeout 吞掉 runc 清理（中）

- `crates/dag-runtime/src/runtime.rs::execute_step`：`sandbox: runc` 的 python 步骤不再套外层 `tokio::time::timeout`——超时所有权归内层 `sandbox/runc.rs::run_step`（其 KILL + `delete --force` 清理尾部必须执行，外层先触发会 drop 整个 future 泄漏容器）。VM（in_process）与 agent 步骤保留外层预算（VM 线程不可打断、超时折叠 Error 为文档化行为，见运维注意）。

### #4 取消排空丢工件（中）

- `runtime.rs` 取消分支：drain 改走与主循环相同的 `record_step`（写 `output.txt/output.json/meta.json` + 发 `step_done` 帧），替换原先只写 states 的 `drain_inflight`（已删除）——`step_io.rs` "never a dangling step" 契约对取消路径成立。

### #5 终态 run 事件追加 + 状态/终帧两事务窗口（中）

- `crates/web/src/api_nodes_dag.rs::post_events`：run 已终态（done/error/cancelled）→ 409 拒绝整批，终帧后不可再插帧。
- 新增 `Store::finalize_dag_run`（`dag.rs::finalize_run`）：单个 `BEGIN IMMEDIATE` 事务内完成 transition 校验 + UPDATE status/finished_at + INSERT 合成 `run_finished` 帧，返回帧 seq——`post_status` 的"状态提交与终帧"两事务崩溃窗口消除。
- `dag.rs::converge_lost`：失联收束在同一事务内为每个收敛 run 插入 `{"status":"error","error":"node lost"}` 帧，返回 `Vec<ConvergedDagRun{record, run_finished_seq}>`（types.rs 新增）；`api_nodes.rs` lost sweep 改为仅 DagHub publish（帧已随事务持久化，publish 失败仅影响在线订阅者、帧仍可从 store 回放）。
- `emit_run_finished` 拆分为 `publish_run_finished`（纯 publish，携带 seq）；`update_dag_run_status` 保留。

## 测试清单

- `crates/dag-runtime`（26 passed）：`tests/run_loop.rs` 新增 `sibling_completion_never_redispatch_inflight_step`（#1 盲区用例：fast 完成后 300ms 内 slow 的 `step_started` 恰 1 次、`call_count()==2`、金丝雀脚本不消耗；已实测回退 filter 即变红）与 `cancel_drain_persists_inflight_step_artifacts_and_frames`（#4：取消后 slow 的 `step_done` 帧 + `meta.json outcome=="cancelled"`；回退为 state-only drain 即变红）。
- `crates/store`（191 passed）：`tests/dag_store.rs` 新增 `claim_blocked_while_cancelling`（#2）、`finalize_commits_status_and_frame_atomically`（#5：seq>0、恰一帧且 seq 匹配、error 透传、重复 finalize 报 illegal、ghost 报 not found）；`lost_sweep_converges_running_and_cancelling` 强化为逐 run 断言 `run_finished_seq>0` 与持久化帧。
- `crates/web`（280 passed）：`tests/dag_api.rs` 新增 `events_rejected_after_terminal_status`（200→终态→409 且不再落帧）；`store_error_surfacing.rs` 适配新签名。
- 修复真实性验证：#1/#4 各自临时回退→对应用例变红→还原后全绿。

## 运维注意（评审假设 A1/A3 如实标注）

- **A1 运行时约束**：`sandbox: in_process` python 步骤不可中途取消——VM 跑在 detached blocking 线程上，超时/取消只是把步骤折叠为 Error/Cancelled，线程会继续运转至代码自然结束（或进程退出），解释器循环无协作取消钩子（`exec/python/mod.rs` 模块注释明示）。runc 路径自 #3 修复后超时即 KILL 容器，无此约束。
- **A3 部署前提**：runc 模式生产可用性依赖运维自备 python 解释器 rootfs——`opencode-agent dag prepare-rootfs` 只输出脚手架模板（`sandbox/oci.rs` 的 rootfs 校验要求真实目录、拒绝符号链接）。
- daemon_smoke 为进程级 e2e，机器高负载（load >100）时 20s 节点注册窗口可能 flake；单独复跑通过。

## 全量回归（2026-09-05 实测）

- workspace `cargo test --workspace --no-fail-fast`：**302 套件 / 4418 passed / 0 failed**（较初测 +2，为并行团队后续合入用例；提交前复跑同数字全绿）（含全部 DAG 相关套件、进程级 daemon_smoke/nodes_smoke、schema_bootstrap）。
- `cargo clippy -p opencoder-dag-runtime -p opencoder-store -p opencoder-web --all-targets`：0 告警；`cargo fmt -- --check` 通过。
- 备注：高负载（load >100，多团队共树并行）时段 `daemon_smoke`（20s 节点注册窗口）与 `schema_bootstrap` 曾出现时序性 flake，两者单独复跑均绿，与本次改动无关（本次改动不触及 schema/迁移与节点注册路径）。
