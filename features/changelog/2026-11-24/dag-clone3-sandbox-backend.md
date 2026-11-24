# DAG 原生 clone3 沙箱后端：`dag.sandbox` 选择、运行回执与统一清理

一句话：新增不依赖 OCI 运行时的原生 clone3 沙箱后端；节点通过
`dag.sandbox`（`runc` | `auto` | `clone3`）选择后端，运行回执记录后端身份，
步骤中继、取消、节点重启恢复与清理对两种后端一致；e2e 矩阵与两节点验收在
runc 与 clone3 两个后端上全部通过。

## 背景

- 既有 DAG 运行要求每个节点安装 `runc`（"runc is required for every DAG
  run"），沙箱形态由 OCI 运行时决定。
- 内核 5.13+ 已能以 `clone3`、`pidfd`、`setns` 直接建立同等的 PID / mount /
  UTS / IPC 隔离，无需外部运行时；后端此前在仓库中并不存在，本记录覆盖其
  从零实现与接线。
- 用户决策：按 clone3 e2e 测试计划引入原生后端作为 `dag.sandbox` 的可选项，
  缺省仍为 `runc`、既有节点行为不变；规则 04 的「单容器」不变量不变，仅把
  文字从 `runc` 容器对齐为「一个沙箱容器」。

## 变更

### 配置（`crates/core`）

- `config/runtime/dag.rs`：`DagSandbox { Runc（默认）, Auto, Clone3 }`，
  `dag.sandbox` 小写序列化、缺省为 `runc`，既有节点配置零变化；合并逻辑
  （`config/merge.rs`）同步 `sandbox`。`DagSandbox::from_test_env()` 读取
  `DAG_TEST_SANDBOX`（取值同配置，缺省 `runc`），供原生测试基建统一钉后端。

### 后端（`crates/dag-runtime/src/sandbox/clone3/`）

- `mod.rs`：`select()` 解析节点配置——`clone3` 校验内核 ≥ 5.13 与 root，
  不满足即拒绝（fail-closed）；`auto` 在可用时选 clone3 否则 `runc`；`runc`
  从不回退。`Backend` 回执枚举、`Identity`（pid + `/proc/<pid>/stat`
  starttime）与 pidfd 信号，避免误伤复用的 pid。
- `launcher.rs`：`dag-runner launch <run_root>`——unshare mount/UTS/IPC，
  在运行自身的 rootfs overlay 上应用 OCI 挂载、写 `etc/hostname`、
  `clone3(CLONE_NEWPID | CLONE_PIDFD)`；子进程 chroot、从命名空间内挂 proc、
  掩蔽路径、降权后 exec init。启动器把节点租约（自身 stdin）复制到 fd 3，
  并以 `OPENCODER_SANDBOX_LEASE_FD` 显式命名；init 只在被命名时监视租约，
  runc 的 init 无此条目、永不触发（此前按位置假定 fd 3 导致 runc 容器在
  `runc run --detach` 退出后被拆除）。`private/clone3/init.json` 记录 init 与
  启动器身份；`supervise` 以 pidfd + 租约 POLLHUP 守护。
- `mounts.rs`：OCI 挂载解析、`/proc/sys` 等只读与掩蔽列表、只读根。
- `privilege.rs`：capabilities / uid / gid / no_new_privileges 过渡校验。
- `exec.rs`：`dag-runner exec <root> <key>` 步骤中继——经 `/proc/<init>`
  setns 进入活动命名空间、应用进程规格；stdin 为节点租约，关闭或停止标志
  即终止步骤。
- `launch.rs`：节点侧 spawn / stop / `stop_abandoned`（按身份核对）。
- `process.rs`：启动器与中继共享的 OCI `process` 解析。

### 运行接线（`sandbox/run/*`、`sandbox/supervisor.rs`）

- `container.json` 回执新增 `backend` 与 `runner`（旧记录按 runc 读取）；
  `RunContainer` 按 `start_runc` / `start_clone3` 启动，`execute` 按回执选择
  `runc exec` 或中继；`cleanup` 先停止启动器 / `stop_abandoned`，再处理
  `runc-state`；`runc-state` 目录与 `runc` 可用性要求仅属于 runc 后端，
  原生运行的根目录不产生任何 OCI 运行时状态。
- `supervisor::init`：租约仅在启动器命名描述符时生效（见上）；租约监视只在
  EOF 或租约不可读时结束，租约上的字节与被中断的读取被消费而不中止监视。

### 测试基建

- e2e（`tests/dag_e2e`）：`clone3_fixtures.rs`（后端与跳过策略、宿主侧
  `/proc/<init>` 视图、mountinfo 解析、"已释放"证明）、`clone3_probe.rs`
  （沙箱内 C 探针）、`clone3_environment.rs`、`clone3_lifecycle.rs`、
  `clone3_agent.rs`；`agent_runc.rs` / `dynamic.rs` 改为双后端
  （`backend_ready()`）。`tests/support/fleet_proc.rs` 新增 `agent_env`、
  `spawn_native_with`、`agent_pid`，`native.rs` 钉 `dag.sandbox`。
- dag-runtime / worker 原生夹具注入 `DagSandbox::from_test_env()`；worker
  `runc_fixture::runner()` 按有界祖先目录解析 example（兼容
  `build/<pkg>/<hash>/out` 布局）；`dag_codex_runc` 断言 runc 专属回执，
  非 runc 钉定时跳过；共享夹具 `tests/support/container.rs` 新增
  `assert_runtime_released()`（runc → `runc-state` 为空；clone3 → 无 `runc-state`
  且记录的 init 已退出；auto → 两者任一），`run_loop/event_stream.rs` 与 worker
  `harness/registered.rs` 的释放断言改用它（原断言在原生运行下因目录不存在失败）。
- `crates/dag-runtime/tests/sandbox_environment.rs`：starttime 读取偏移由
  `nth(18)` 修正为 `nth(19)`（原断言读到 itrealvalue，实际为空检查）。

### 验收脚本（`scripts/acceptance/runc_scheduling`）

- `runtime.py`：节点配置写入 `dag.sandbox`（`DAG_TEST_SANDBOX`，缺省
  `runc`）；`close()` 对记录的原生沙箱先等待租约回收、超时按身份 SIGKILL；
  `samples.clean()` 仅在 `runc-state` 存在时查询 `runc list`（查询缺失
  root 会创建目录），并断言记录的 clone3 进程均已退出。

### 规则与仓库记忆

- `rules/04-dag-execution-contract.md`：验收要求、单容器约定与同步清单的
  文字对齐双后端沙箱（不变量不变）；`agents/workflow/dag-runtime/index.md`
  补 `sandbox/clone3/` 条目与后端说明。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| `dag.sandbox` 配置序列化与 `DAG_TEST_SANDBOX` | `sandbox_modes_roundtrip_and_unknown_values_fail`、`test_env_backend_defaults_to_runc_and_accepts_config_values` | `crates/core/src/config/runtime/dag.rs` |
| 后端选择：auto 偏好 clone3、钉定不回退、内核/root 要求 | `auto_prefers_clone3_and_pinned_never_falls_back`、`requirements_pin_the_pidfd_release_and_root`、`kernel_release_versions_parse_like_uname` | `crates/dag-runtime/src/sandbox/clone3/mod.rs` |
| 进程身份：comm 含空格/括号的 stat 解析 | `stat_parsing_skips_comm_with_spaces_and_parens` | `crates/dag-runtime/src/sandbox/clone3/mod.rs` |
| OCI 挂载解析、目的路径规范化、只读/掩蔽保护 | `oci_entries_parse_into_mounts_and_unknown_options_fail`、`destinations_are_absolute_normalized_guest_paths`、`protection_lists_resolve_to_guest_paths` | `crates/dag-runtime/src/sandbox/clone3/mounts.rs` |
| capabilities 解析与降权过渡 | `capabilities_parse_names_and_reject_garbage`、`transition_validates_capabilities_against_the_process_user` | `crates/dag-runtime/src/sandbox/clone3/privilege.rs` |
| 进程规格解析与租约命名 | `process_parses_and_non_root_drops_capabilities`、`process_rejects_incomplete_specs`、`lease_entry_names_the_descriptor_and_replaces_the_bundle_one` | `crates/dag-runtime/src/sandbox/clone3/process.rs` |
| init 阶段错误报告（async-signal-safe） | `init_report_formats_negative_errno_without_allocating` | `crates/dag-runtime/src/sandbox/clone3/launcher.rs` |
| 中继停止宽限期与停止标志 | `grace_is_bounded_and_the_stop_flag_round_trips` | `crates/dag-runtime/src/sandbox/clone3/exec.rs` |
| init 租约监视只在 EOF / 不可读时结束 | `lease_watch_ends_only_at_eof_or_on_an_unreadable_lease` | `crates/dag-runtime/src/sandbox/supervisor.rs` |
| 真实容器 PTY、嵌套 knowledge 只读挂载（双后端） | `real_container_allocates_and_exchanges_data_through_a_pty`、`real_container_reads_nested_knowledge_mounts_but_cannot_write_them` | `crates/dag-runtime/tests/sandbox_environment.rs` |
| 运行器预检拒绝缺失/混版/软链 | `rootfs_rejects_missing_mixed_version_and_linked_runners_before_admission` | `crates/dag-runtime/tests/preflight.rs` |
| 沙箱环境：凭据、能力、命名空间、只读根、掩蔽、挂载（双后端） | `clone3_environment::step_sandbox_is_namespaced_readonly_and_unprivileged` | `tests/dag_e2e/clone3_environment.rs` |
| 宿主侧活动沙箱可见、取消后全部释放（双后端） | `clone3_environment::live_sandbox_is_visible_from_the_host_and_released_on_cancel` | `tests/dag_e2e/clone3_environment.rs` |
| 钉定 clone3 的节点无需 runc 且永不被 runc 替换 | `clone3_lifecycle::pinned_native_backend_needs_no_runc_and_is_never_replaced_by_it` | `tests/dag_e2e/clone3_lifecycle.rs` |
| 节点重启中断运行、resume 复用同一运行（双后端） | `clone3_lifecycle::node_restart_interrupts_the_run_and_resume_reuses_it` | `tests/dag_e2e/clone3_lifecycle.rs` |
| 受管 harness（codex）在沙箱内运行 rootfs 二进制、凭据目录 rw、缺二进制预检拒绝（双后端） | `clone3_agent::harness_agent_step_runs_the_rootfs_binary_inside_the_sandbox` | `tests/dag_e2e/clone3_agent.rs` |
| e2e 夹具：status 字段折叠、mountinfo 两段解析、身份判定 | `clone3_fixtures::tests::status_values_collapse_whitespace_and_mounts_parse_both_halves`、`clone3_fixtures::tests::identity_tracks_the_live_test_process_and_rejects_others` | `tests/dag_e2e/clone3_fixtures.rs` |
| 既有 agent/动态场景在两种后端上一致 | `agent_runc::agent_step_session_runs_inside_runc_container`、`dynamic::*` | `tests/dag_e2e/agent_runc.rs`、`tests/dag_e2e/dynamic.rs` |
| worker runc codex 链路（夹具注入后端） | `server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation` | `crates/worker/tests/dag_codex_runc.rs` |
| 运行结束后不残留沙箱：runc 状态为空 / 原生 init 已退出（双后端） | `run_loop::event_stream::retained_agent_events_are_reset_before_live_import_starts`、`invalid_live_agent_event_is_error_not_user_cancellation`、`harness_matrix::codex_orchestrators_without_native_credentials` | `crates/dag-runtime/tests/support/container.rs`、`crates/dag-runtime/tests/run_loop/event_stream.rs`、`crates/worker/tests/harness/registered.rs` |

## 验证

- e2e 矩阵（`cargo test --test dag_e2e -- --test-threads=1`）：默认（runc）
  17 通过 0 失败；`DAG_TEST_SANDBOX=clone3` 17 通过 0 失败、无跳过。
- `cargo test -p opencoder-dag-runtime` 两种后端各一遍：lib 87 通过（3
  ignored）、dynamic 14、preflight 1、resources 2、run_loop 14、
  sandbox_environment 2，均通过。
- `cargo test -p opencoder-worker --test harness_matrix`：runc 与
  `DAG_TEST_SANDBOX=clone3` 各 1 通过；`--test dag_codex_runc`：runc 通过，
  `DAG_TEST_SANDBOX=clone3` 下按策略跳过。
- `cargo clippy -p opencoder-core -p opencoder-dag-runtime --all-targets` 与
  `-p opencoder -p opencoder-worker --tests`，`-D warnings` 零告警。
- 两节点验收 `scripts/acceptance/runc_scheduling/main.py`（`--bin-dir
  target/debug --rootfs target/debug/dag-rootfs`，证据在
  `/root/.cache/opencoder-e2e/2026-11-25-{clone3-3,runc-2}`，收尾改动后重跑）：
  两种后端均 `passed: true`——20 个原生执行样本（clone3 冷启动 p95 0.39 s /
  runc 0.51 s）、失败与超时、取消、3 个动态实例、16 MiB 产物、Worker 重启后
  的固定版本恢复；回执后端分别为 `clone3` / `runc`（各 28 份），源工作区不变，
  清理后无剩余挂载，三个进程正常退出；原生运行根下无 `runc-state`（runc 根下
  28 个）。
- 测试范围：本次修改及其直接影响的链路（dag-runtime 全部 target、core 配置、
  e2e `dag_e2e`、worker `harness_matrix` 与 codex runc 用例、两节点验收脚本）；
  未运行全量 workspace 测试与 SPA 测试。
