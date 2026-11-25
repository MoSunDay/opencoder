# DAG 沙箱宿主映射收尾：节点数据目录上的只读 tmpfs `/`、运行器目录与 rootfs 路线清理

一句话：按用户裁决「不隔离 fs」重做 DAG 沙箱的根——沙箱 `/` 不再是按次复制的 rootfs 镜像，
也不是宿主 `/`，而是节点数据目录下本次运行目录中的空锚点（`bundle/root`）上由运行时在私有
挂载命名空间内挂起的只读 tmpfs，宿主 `/bin /etc /lib /lib64 /sbin /usr`（含子挂载）、
`/etc/hosts`、`/etc/resolv.conf` 与运行器目录（`/run/opencoder/bin`）逐条只读绑定到其上；
步骤直接使用宿主的解释器与工具，镜像只剩运行器。旧的 OCI/rootfs 复制、`opencoder-agent dag
prepare-rootfs`、镜像安装脚本与 `#[ignore]` smoke 一并删除；runc 与 clone3 双矩阵、dag-runtime、
worker、project 与平台维护预检全部通过，产物 / NFS 等存储协议与 `/workspace`、知识库、
Agent、私有文件、Codex 凭证挂载不变。

## 背景与用户决策

- 用户裁决：**不隔离 fs**——步骤直接使用宿主文件系统，但沙箱内 `/` 必须是节点数据目录下的
  路径（非宿主 `/`），用一张简单路径映射实现；不引入 `node_data_dir` 配置字段，运行目录
  本身就是节点数据目录下的路径，根锚点取运行目录内的 `bundle/root`。
- 用户随后要求「完成 100% 收尾」：按推荐默认与路线 B 全部落地并收口——运行时 → 测试夹具 /
  e2e → 清脚本 / CI / CLI → 规则 04 / 记忆 / changelog → 提交。
- 规则 04 的单容器模型、同一 `/workspace`、受理时冻结配置、产物声明与归档协议全部不动；
  本次只替换根文件系统的来源，并把运行器目录从「镜像内固定路径」改为配置键。

## 变更

### 运行时（crates/dag-runtime）

- `sandbox/host.rs`（新增）：宿主映射的纯函数表——`HOST_DIRECTORIES`（`/bin /etc /lib
  /lib64 /sbin /usr`，按挂载顺序）、`HOST_FILES`（`/etc/hosts`、`/etc/resolv.conf`，存在才绑）、
  `RUNNERS_MOUNT`（`/run/opencoder/bin`）与 `GUEST_PATH`；`runners_dir(config)` 解析
  `dag.runners_dir` → `<dag.rootfs_dir>/usr/bin` → 否则报 `DAG runners_dir is required`
  （没有 `current_exe()` 回退）；`mounts(mountinfo, host, runners)` 以只读 tmpfs `/`
  （`nosuid,nodev,noexec,mode=755,size=64m`）为第一个 OCI 挂载，宿主目录的软链接源
  canonicalize 后非 rbind 绑定（merged-usr 下 `/bin` → `/usr/bin` 仍绑到 guest 名），嵌套
  子挂载逐条只读绑定，文件类源在私有命名空间内沿链创建占位文件后绑定（本机
  `/etc/resolv.conf` 经 systemd stub 链仍可用，DNS 保留）；`host_path` 把 guest 路径翻译到
  宿主（`/run/opencoder/bin/...` 翻译到运行器目录），`validate_executable` 据此校验 Codex
  可执行文件与启动脚本。
- `sandbox/run/{config.rs,mounts.rs,mod.rs}`：OCI `root.path="root"` 指向空锚点
  `<run_root>/bundle/root`；挂载表由 `host::mounts` 开头，其后是 `/workspace` 写层、
  `/run/opencoder-private`、知识库（含嵌套子挂载的浅绑定）、Agent、私有文件与 Codex 凭证
  挂载；不再创建 rootfs 目录树或挂载点占位。
- `sandbox/clone3/{mounts.rs,launcher.rs,launch.rs,mod.rs}`：原生后端按同一 OCI 表挂载；
  `root.readonly` 由 `seal_root` 在全部条目挂好后以 `MS_BIND|MS_REMOUNT|MS_RDONLY` 重挂
  tmpfs 顶挂载实现——不能用 self-bind，它会把子挂载从根的查找路径上遮住，init 随即在
  `protect proc` 处失败（errno 2）；tmpfs 属于沙箱私有命名空间，节点挂载表里看不到。
- `sandbox/codex/{mod.rs,startup.rs}`：`resolve(config, agent, host_root())` 经宿主映射解析
  profile `executable`（默认 `/usr/bin/codex`）与启动脚本，只接受宿主映射路径上的真实可执行
  文件（`/usr/...` 或 `/run/opencoder/bin/...`），拒绝 `/tmp /proc /dev /run/opencoder-private
  /workspace`；缺失时报 `Codex executable missing on the host at <path>; install it under /usr
  or dag.runners_dir`；`codex/bundle.rs` 与 `guest_path` 删除。
- 删除：`sandbox/oci.rs`、`sandbox/rootfs.rs`（rootfs 校验与按次复制）、`sandbox/runc.rs`
  三个 `#[ignore]` smoke 与无调用的 `run_step`（`run_step_cancellable/streamed` 仍由 worker
  Brain 使用）、`exec/mod.rs` 的 `KNOWLEDGE_MOUNT/AGENTS_MOUNT` 旧常量。
- `crates/dag-runtime/examples/`：删除 `codex-startup-probe.rs`、`connection-mount-probe.rs`、
  `container-check.rs`。

### 配置与入口

- `crates/core/src/config/runtime/dag.rs`、`merge.rs`：新增持久化键 `dag.runners_dir`（进键表
  与合并测试）；`dag.rootfs_dir` 保留，语义收窄为「镜像目录」，是平台镜像替换
  （`image.rs`）唯一改写的键；`runners_dir` 未配置时取 `<rootfs_dir>/usr/bin`。
- `crates/agent`：删除 `opencoder-agent dag prepare-rootfs` 子命令与 `opencoder-dag-runtime`
  依赖（`Cargo.lock` 随之更新）。
- `crates/worker/src/operations/dag_preflight.rs`：受理前以 `host_root()` 校验运行器目录；
  `crates/project`：DAG 恢复路径随 `resolve` 签名调整。

### 脚本、CI 与文档

- `scripts/prepare-dag-rootfs.sh`：不再安装 Python、工具与 Codex，只构建三个运行器示例
  （`dag-runner`、`agent-step-runner`、`agent-session-runner`）装入 `<out>/usr/bin` 并提示
  `dag.runners_dir = <out>/usr/bin`；`scripts/dag-rootfs/*`（`install-codex*.sh`、
  `install-python.sh`、`install-tools.sh`、`common.sh`）删除。
- `.github/workflows/brain-e2e.yml`、`scripts/ci/brain.py`、`scripts/acceptance/`
  （`harness/native.js`、`ui/main.js`、`node_sigkill.js`、`runc_scheduling/{runtime.py,cases.py}`、
  `brain/history/main.py`）：夹具改为「每测试一份运行器目录 + 伪造 Codex CLI /
  `codex-start.sh`」，profile `executable=/run/opencoder/bin/codex`；参数名 `--rootfs`、
  `DAG_TEST_ROOTFS`、脚本名与 `dag.rootfs_dir` 保留，语义统一为「含 `usr/bin/` 运行器的
  镜像目录」。
- `scripts/platform/rolling/maintenance/preflight.py`：配置键白名单加入 `runners_dir`。
- 规则 04：运行器目录改为配置键 `dag.runners_dir`（回退 `<dag.rootfs_dir>/usr/bin`），新增
  沙箱 `/` 契约（空锚点、tmpfs、宿主只读映射、Codex 安装位置限制、宿主锚点保持为空）；
  维护升级的互不包含检查加入运行器目录。模块索引、DAG 功能卡、Harness 卡、
  `rules/05-ui-acceptance.md`、两份验收 README 与 `repo-memory.md` 同步。
- Ontology（规则 09）核对：`crates/core/src/ontology/platform.rs` 只登记 `opencoder-cli` 的
  HTTP 域（`dag` → `/api/dag`、`/api/workflows`，未变）；`opencoder-agent dag prepare-rootfs`
  与 `dag.*` 配置键均不在登记内，无需同步。
- 刻意未动：`scripts/platform/rolling/backup_files/scope.py` 及其夹具（备份范围按目录名，
  不依赖镜像内容）、`scripts/acceptance/smooth_release/containers.py`、
  `skills/memory-snapshots/`（冻结快照）、`crates/dag/src/artifacts.rs` 的保留名
  `rootfs`（SPA 镜像目录名）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行器目录解析：显式键优先，否则镜像目录 `usr/bin`，都没有则拒绝 | `host::tests::runners_directory_prefers_the_explicit_key_then_the_image_usr_bin` | `crates/dag-runtime/src/sandbox/host.rs` |
| guest 路径经运行器挂载翻译到运行器目录；映射表外路径拒绝 | `host::tests::guest_paths_below_the_runners_mount_name_the_runners_directory` | `crates/dag-runtime/src/sandbox/host.rs` |
| 可执行文件经运行器目录校验：缺执行位、无目录、缺失、非映射路径各自报错 | `host::tests::executables_are_validated_through_the_runners_directory` | `crates/dag-runtime/src/sandbox/host.rs` |
| 受理前拒绝缺失、混用版本与软链接的运行器 | `runners_dir_rejects_missing_mixed_version_and_linked_runners_before_admission` | `crates/dag-runtime/tests/preflight.rs` |
| 缺少运行器目录在持久化受理前拒绝 | `admission_tests::missing_runners_dir_is_rejected_before_durable_acceptance` | `crates/worker/src/operations/admission_tests.rs` |
| 预检拒绝缺失或不完整的运行器镜像 | `cancel_fail::preflight_rejects_missing_or_incomplete_runners_image` | `tests/dag_e2e/cancel_fail.rs` |
| `dag.runners_dir` 进入持久化键表与合并 | `config::merge::tests::merge_dag_block_binary_dir_and_nfs` | `crates/core/src/config/merge.rs` |
| 真实容器启动并经 PTY 交换数据、只读读到嵌套知识库挂载；宿主上的根锚点保持为空 | `real_container_allocates_and_exchanges_data_through_a_pty`、`real_container_reads_nested_knowledge_mounts_but_cannot_write_them` | `crates/dag-runtime/tests/sandbox_environment.rs` |
| 沙箱根是 tmpfs，宿主映射与运行器目录只读，init 来自 `/run/opencoder/bin/dag-runner`；init 的根既非宿主 `/` 也非锚点；宿主锚点前后为空；主机名与宿主一致 | `clone3_environment::step_sandbox_is_namespaced_readonly_and_unprivileged` | `tests/dag_e2e/clone3_environment.rs` |
| 运行中沙箱从宿主可见、取消后释放，锚点仍为空 | `clone3_environment::live_sandbox_is_visible_from_the_host_and_released_on_cancel` | `tests/dag_e2e/clone3_environment.rs` |
| Agent 步骤的 harness 以 `/run/opencoder/bin/...` 在运行器目录内解析并在沙箱内运行 | `clone3_agent::harness_agent_step_runs_the_runners_image_binary_inside_the_sandbox` | `tests/dag_e2e/clone3_agent.rs` |
| Codex 步骤在 runc 沙箱内完成会话 | `agent_runc::agent_step_session_runs_inside_runc_container`、`dag_spec_dispatch_runs_binary_and_agent_steps_to_done` | `tests/dag_e2e/agent_runc.rs`、`tests/dag_e2e/flow.rs` |
| 取消运行与 Agent 步骤失败 | `cancel_fail::cancel_mid_run_and_agent_step_failure` | `tests/dag_e2e/cancel_fail.rs` |
| Codex 步骤在 runc 沙箱内经伪造 CLI 完成 | `dag_codex_runc` | `crates/worker/tests/dag_codex_runc.rs` |
| 镜像目录替换后恢复固定版本 | `dag_image_recovery` | `crates/worker/tests/dag_image_recovery.rs` |
| Worker 矩阵：DAG-only 文件 Agent 经沙箱执行 | `harness_matrix` | `crates/worker/tests/harness_matrix.rs` |
| 受理钉住配置、丢失驱动先清理再进终态 | `dag_recovery::dag_acceptance_pins_config_and_lost_driver_cleanup_precedes_terminal_status` | `crates/project/src/service_tests/dag_recovery.rs` |
| Codex profile 解析校验宿主可执行文件 | `codex::tests::profile_resolution_validates_host_binary_and_private_profile` | `crates/dag-runtime/src/sandbox/codex/tests.rs` |
| 平台维护预检接受 `runners_dir`、拒绝未知 `dag` 字段 | `test_runners_dir_is_accepted_while_unknown_dag_fields_are_rejected` | `scripts/platform/maintenance_tests/test_preflight.py` |

## 验证

- e2e 双矩阵（`cargo test -p opencoder --test dag_e2e -- --test-threads=1`）：
  默认 runc **18 passed / 0 failed / 0 ignored**（233 s）；`DAG_TEST_SANDBOX=clone3`
  **18 passed / 0 failed / 0 ignored**（240 s）。首轮 runc 矩阵 17 passed / 1 failed：
  `clone3_agent` 被 preflight 以 400 拒绝，原因是 `target/debug/opencoder-agent` 仍是
  `host_path` 修复前构建的旧二进制（`cargo test --test dag_e2e` 不重建兄弟二进制）；
  `cargo build -p opencoder-agent -p opencoder-server -p opencoder-cli` 后重跑两个矩阵全绿，
  实现代码未再改动。
- `cargo test -p opencoder-dag-runtime` 双后端各一遍：lib 77 passed、dynamic 14、preflight 1、
  resources 2、run_loop 14、sandbox_environment 2，均通过。
- `cargo test -p opencoder-worker`：`--test dag_codex_runc`、`--test harness_matrix`、
  `--test dag_image_recovery` 双后端各 1 passed。
- `cargo test -p opencoder-project --lib dag_recovery` 1 passed；`cargo test -p opencoder-agent`
  35 passed。
- `/usr/bin/python3 -m unittest scripts/platform/maintenance_tests/test_preflight.py`：8 个用例
  通过（含 `runners_dir` 用例）。
- `cargo clippy -p opencoder-dag-runtime -p opencoder-agent -p opencoder-worker -p opencoder-project
  --all-targets -- -D warnings` 与 `cargo clippy -p opencoder --test dag_e2e -- -D warnings`
  零告警；`rustfmt --edition 2021` 对本轮 .rs 文件全部干净。
- 测试范围：本次改动及其直接影响链路（dag-runtime 双后端、e2e 双矩阵、worker 沙箱 / 矩阵 /
  镜像恢复、project 恢复、agent CLI、平台维护预检）；未运行全量 workspace 测试、SPA 测试与
  两节点验收脚本。

## 已知取舍

- 沙箱内 `/etc` 就是宿主只读 `/etc`（含 root-only 文件；Agent 持 `CAP_DAC_OVERRIDE` 可读），
  这是「不隔离 fs」裁决的直接后果，规则 04 已写明。
- 运行器目录没有 `current_exe()` 回退：显式键与镜像目录都没有时报错，避免节点在二进制池
  版本与运行器版本不一致时静默启动。
- 旧名字（`--rootfs`、`DAG_TEST_ROOTFS`、`prepare-dag-rootfs.sh`、`dag.rootfs_dir`）全部保留，
  避免验收脚本与平台发布链路的参数改名。
