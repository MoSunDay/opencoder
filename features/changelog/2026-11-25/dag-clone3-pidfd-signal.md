# DAG clone3 收尾：pidfd 精确信号、`/etc/hostname` 双后端对齐、溯源戳刷新

一句话：clone3 清理对记录的 init 改经 pidfd 发送信号（pidfd 钉住进程后再核对 starttime），
pidfd 可用时消除身份检查与信号之间的 pid 复用窗口，不可用时告警并回退身份守卫；节点为两个后端
统一在运行自身的 rootfs overlay 内落盘 `/etc/hostname`；规则 04、dag-runtime 索引与 DAG 功能卡的
`Commit:` 溯源戳刷新。

## 背景

- `dd29777be` 评审的问五指出 `clone3::kill` 为「存活检查 + `kill(pid)`」，检查与信号之间
  仍有理论上的 pid 复用窗口。
- 用户决策：首轮收尾（`8edc26f26`）免测提交并授权推送远端——不运行 dag-runtime 全 target、
  e2e 双矩阵、worker 矩阵与两节点验收；因此首轮不宣称该增量可上线，可上线基线仍是 `dd29777be`。
  本次提交按评审问五补齐 P3 项并跑完上述门槛（见验证）；门槛全部通过，可上线基线由
  `dd29777be` 前移至本提交。
- 问五中的 `/etc/hostname` 双后端对齐：e2e `step_sandbox_is_namespaced_readonly_and_unprivileged`
  在两个矩阵里观测到两后端 UTS 主机名同为 `dag-run`，只有原生启动器把主机名落盘为
  `/etc/hostname`，runc 容器内没有该文件。本次决定由节点统一落盘，启动器不再自行写入。

## 变更

- `crates/dag-runtime/src/sandbox/clone3/mod.rs`：`kill()` 先 `pidfd_open` 钉住进程，再读
  starttime 核对身份，经 `pidfd_send_signal` 发送；pidfd 不可得（非 ESRCH，如描述符耗尽）
  时记录警告并回退到原「身份检查 + `kill`」路径；进程已退出（ESRCH）不是错误。身份读取移出
  unsafe 块；新增仅测试可见的回退计数，单据此断言 pidfd 路径确实被走到、pid 0 走回退。
- `crates/dag-runtime/src/sandbox/run/mod.rs`：`prepare()` 在挂好 rootfs overlay 之后，把 OCI
  `hostname` 以 `<host>\n` 写入 `bundle/rootfs/etc/hostname`（镜像含 `etc/` 时），runc 与
  clone3 两个后端共用；只写运行自己的 upper 层，镜像不变。
- `crates/dag-runtime/src/sandbox/clone3/launcher.rs`：删除启动器内的 `/etc/hostname` 写入，
  `sethostname` 保持不变。
- `tests/dag_e2e/clone3_environment.rs`：`etc_hostname` 断言改为双后端无条件 `dag-run`。
- `rules/04-dag-execution-contract.md`、`agents/workflow/dag-runtime/index.md`：`Commit:`
  溯源戳刷新为 `dd29777be`（首轮）；`features/platform/dag/index.md`：措辞补上 clone3 后端，
  溯源戳刷新为 `8edc26f26`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 只信号记录的那一次进程化身；陈旧身份与已退出进程均无副作用；pidfd 路径不回退、pid 0 走回退且不信号 | `kill_reaches_the_recorded_incarnation_only` | `crates/dag-runtime/src/sandbox/clone3/mod.rs` |
| 步骤沙箱内 UTS 主机名与 `/etc/hostname` 在 runc、clone3 两个矩阵下同为 `dag-run` | `step_sandbox_is_namespaced_readonly_and_unprivileged` | `tests/dag_e2e/clone3_environment.rs` |

## 验证

- e2e 矩阵（`cargo test -p opencoder --test dag_e2e -- --test-threads=1`）：默认（runc）
  17 通过 0 失败；`DAG_TEST_SANDBOX=clone3` 17 通过 0 失败；两个矩阵均无跳过——首轮
  按免测决策未跑的项已补跑闭环，`etc_hostname == "dag-run"` 断言对两个后端无条件生效，
  并在两个矩阵中实际执行。
- `cargo test -p opencoder-dag-runtime` 两种后端（`DAG_TEST_SANDBOX` 默认 runc 与
  `clone3`）各一遍：lib 88 通过（3 ignored）、dynamic 14、preflight 1、resources 2、
  run_loop 14、sandbox_environment 2，均通过；其中 `sandbox::clone3::tests` 5 个
  （含新增的 pid-0 回退断言）。
- `cargo test -p opencoder-worker --test harness_matrix`：runc 与 `DAG_TEST_SANDBOX=clone3`
  各 1 通过。
- `cargo clippy -p opencoder-dag-runtime --all-targets -- -D warnings` 与
  `cargo clippy -p opencoder --test dag_e2e -- -D warnings` 零告警。
- 两节点验收 `scripts/acceptance/runc_scheduling/main.py`（`--bin-dir target/debug
  --rootfs target/debug/dag-rootfs`，证据在
  `/root/.cache/opencoder-e2e/2026-11-25-{runc-3,clone3-4}`）：两种后端均
  `passed: true`——各 20 个原生执行样本（runc 冷启动 p95 0.30 s / clone3 0.24 s）、
  失败与超时（4 个失败步骤）、1 个取消运行、3 个动态实例、16 MiB 产物、Worker 重启后
  的固定版本恢复；回执后端分别为 `runc` / `clone3`（各 28 份），源工作区不变，清理后
  无剩余挂载，三个进程退出码均为 0；原生运行根下无 `runc-state`（runc 根下 28 个）。
- 测试范围：本次改动及其直接影响链路（dag-runtime 全部 target 双后端、e2e `dag_e2e`
  双矩阵、worker `harness_matrix` 双后端、两节点验收双矩阵、上述定向 clippy）；未运行
  全量 workspace 测试与 SPA 测试。
