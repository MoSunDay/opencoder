# DAG 沙箱 e2e 补齐：PID 隔离、工作区共享与宿主网络共享

一句话：新增双后端 e2e，以可观测行为确认 DAG 沙箱只隔离进程——同一运行的步骤共享
PID 命名空间（与节点不同）、共享可写 `/workspace`，并直接访问节点网络；runc 与
clone3 两个矩阵、dag-runtime、worker 与两节点验收全部通过，产物 / NFS 等存储协议
未被打破。

## 背景

- 规则 04 的单容器模型：一次运行只启动一个沙箱，静态步骤、动态实例与 Agent 步骤
  共享同一个 `/workspace`；目录是组织方式，不是步骤之间的安全边界，不得加入步骤级
  运行时隔离。
- clone3 后端通过 `dag.sandbox` 替换 runc：`clone3` 钉定时不需要 OCI 运行时（预检
  不再要求 runc，PATH 中无 runc 也能完成运行），`auto` 在内核 ≥ 5.13 且 root 时优先
  clone3，`runc` 从不回退；缺省仍为 `runc`，既有节点行为不变。
- 既有 e2e 用命名空间 inode 与挂载表证明隔离边界，但缺少三类功能性断言：步骤是否
  真能连到节点 loopback 服务、两个步骤是否真能经 `/workspace` 交接文件、工作区写层
  是否落到运行目录。

## 变更

- `tests/dag_e2e/sandbox_sharing.rs`（新增）：一个双后端场景——
  - 测试进程在 `127.0.0.1` 起一个 loopback 应答器；
  - `writer` 步骤把交接文件写入 `/workspace/netfs-handoff.txt`，连接应答器并打印
    回复与自身 PID 命名空间；
  - `reader` 步骤依赖 `writer`，读取同一文件并打印自身 PID 命名空间；
  - 断言两个步骤的 PID 命名空间相同、且不同于测试进程（节点）的 PID 命名空间；
    断言网络回复与文件交接成功；断言交接文件落在运行目录的 `upper/` 写层。
- `tests/dag_e2e/main.rs`：注册 `sandbox_sharing` 模块。
- 评审收尾加固（仅测试代码）：writer 步骤循环 `read` 至 EOF，TCP 分片不再可能让
  `net:` 断言偶发假阴性；loopback 应答器等待连接设 300 s 上限（std 的 `TcpListener`
  没有读超时接口，以非阻塞轮询实现；高于夹具 `wait_terminal` 的 180 s 等待上限），
  运行未到达 writer 步骤时线程不会永久阻塞。

无实现改动：本次只补齐 e2e 覆盖并复核既有 clone3 / runc 后端。

## 功能 → 测试

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 同一运行的步骤共享 PID 命名空间且与节点隔离 | `sandbox_sharing::steps_share_the_workspace_and_the_node_network` | `tests/dag_e2e/sandbox_sharing.rs` |
| 步骤共享可写 `/workspace`（跨步骤文件交接） | 同上 | `tests/dag_e2e/sandbox_sharing.rs` |
| 沙箱使用节点网络（无 network namespace，loopback 可达） | 同上 | `tests/dag_e2e/sandbox_sharing.rs` |
| 工作区写层落到运行目录（存储协议未被打破） | 同上 | `tests/dag_e2e/sandbox_sharing.rs` |
| clone3 替换 runc：钉定 clone3 的节点无 runc 也完成运行且永不回退 | `clone3_lifecycle::pinned_native_backend_needs_no_runc_and_is_never_replaced_by_it` | `tests/dag_e2e/clone3_lifecycle.rs` |

## 验证

- e2e 双矩阵（`cargo test -p opencoder --test dag_e2e -- --test-threads=1`）：
  默认 runc **18 passed / 0 failed**（clone3-only 的钉定用例按策略跳过）；
  `DAG_TEST_SANDBOX=clone3` **18 passed / 0 failed**，无跳过；新增用例在两个矩阵
  均实际执行。
- 收尾加固后定向复跑（只跑本次改动的用例）：
  `cargo test -p opencoder --test dag_e2e sandbox_sharing -- --test-threads=1` 双后端各
  1 passed / 0 failed；`cargo fmt --all -- --check` 与
  `cargo clippy -p opencoder --test dag_e2e -- -D warnings` 通过。其余条目为加固前的
  实测记录，加固未改动实现代码与断言。
- `cargo test -p opencoder-dag-runtime` 双后端各一遍：lib 88 passed（3 ignored）、
  dynamic 14、preflight 1、resources 2、run_loop 14、sandbox_environment 2，均通过。
- `cargo test -p opencoder-worker --test harness_matrix` 双后端各 1 passed；
  `--test artifact_stream` 2 passed（声明产物归档/下载协议）；
  `--test nfs_mount -- --ignored` 1 passed（手工 NFS 只读挂载协议，本机具备 NFS 客户端）；
  `--test dag_codex_runc` 1 passed（runc 矩阵）。
- `cargo test -p opencoder-dag`：lib 31 passed、failure_policies 2 passed（产物声明与
  协议校验）。
- 两节点验收 `scripts/acceptance/runc_scheduling/main.py`（`--bin-dir target/debug
  --rootfs target/debug/dag-rootfs`）双后端各一遍：证据
  `/root/.cache/opencoder-e2e/2026-11-25-{runc-4,clone3-5}`，均 `passed: true`——
  各 20 个原生执行样本（runc 冷启动 p95 0.308 s / clone3 0.433 s）、4 个失败步骤、
  1 个取消运行、3 个动态实例、16 MiB 产物、Worker 重启后的固定版本恢复；清理后源
  工作区不变、无剩余挂载、三个进程退出码均为 0。
- `cargo fmt --all -- --check` 通过；`cargo clippy -p opencoder --test dag_e2e -- -D warnings`
  零告警。
- 测试范围：本次改动（新增 e2e）及其直接影响链路（dag_e2e 双矩阵、dag-runtime 双后端、
  worker 沙箱/产物/NFS 定向用例、两节点验收双矩阵）；未运行全量 workspace 测试与 SPA
  测试。
