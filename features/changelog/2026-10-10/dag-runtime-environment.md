Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# DAG 容器依赖、事件恢复与 Brain 物理节点约束

容器 `/dev` 缺少 devpts 导致 PTY 分配失败；知识根的单层 bind 隐藏了嵌套运行资源。容器现在建立独立 devpts，并将知识根内的目录和文件子挂载逐项只读映射，其他资源根保持单层 bind。

容器验证还发现镜像制备脚本固定选择了宿主 Python 3.7，缺少 `copytree(dirs_exist_ok)` 和 `Path.is_relative_to`。制备入口支持 `--python`，要求 Python ≥ 3.11，保留解释器原布局及随附动态库，并安装系统 CA 和时区。校验阶段在独立挂载命名空间提供设备；真实 DAG 中继续使用容器设备。

Codex 镜像制备同时安装对应目录的 `codex-code-mode-host`，并补齐 `rg`、`printenv`。两节点分别完成真实模型工具调用验证，登录来源与冻结 profile 保留。

恢复 Agent 步骤时，事件读取器曾先读到旧流，随后生产者截断文件，旧偏移落入新 JSON 行中间；内部取消又将持久化错误误记为 Cancelled。现在启动双方之前安全重置工作副本的普通事件文件，拒绝软链接，并只取消内部子进程；不改旧终态或放宽事件解析。

Brain 普通子执行继承根请求显式的物理 `request.node_id`；未指定时仍独立调度，不读取根索引的选点或计划逻辑节点字段。定向测试覆盖显式物理节点的保留和未指定时的独立调度。

## 测试覆盖

| 功能 | 测试名或入口 | 文件 |
| --- | --- | --- |
| 子挂载边界、顺序和去重 | `knowledge_submounts_are_bounded_ordered_and_deduplicated` | [mounts.rs](../../../crates/dag-runtime/src/sandbox/run/mounts.rs) |
| mountinfo 转义只解析一次 | `knowledge_submount_paths_decode_kernel_escapes_once` | [mounts.rs](../../../crates/dag-runtime/src/sandbox/run/mounts.rs) |
| devpts 与无子挂载时的 bind 范围 | `run_uses_shallow_binds_when_knowledge_has_no_submounts` | [config_tests.rs](../../../crates/dag-runtime/src/sandbox/run/config_tests.rs) |
| 真容器 PTY 分配和传输 | `real_container_allocates_and_exchanges_data_through_a_pty` | [sandbox_environment.rs](../../../crates/dag-runtime/tests/sandbox_environment.rs) |
| 嵌套目录/文件只读、工作区可写与清理 | `real_container_reads_nested_knowledge_mounts_but_cannot_write_them` | [sandbox_environment.rs](../../../crates/dag-runtime/tests/sandbox_environment.rs) |
| 镜像解释器、系统 CA、ZoneInfo | 显式 Python 3.11 的安装与 chroot 校验 | [install-python.sh](../../../scripts/dag-rootfs/install-python.sh) |
| 旧事件流重置 | `retained_agent_events_are_reset_before_live_import_starts` | [event_stream.rs](../../../crates/dag-runtime/tests/run_loop/event_stream.rs) |
| 持久化错误不变成用户取消 | `invalid_live_agent_event_is_error_not_user_cancellation` | [event_stream.rs](../../../crates/dag-runtime/tests/run_loop/event_stream.rs) |
| 拒绝软链接且不截断目标 | `event_stream_reset_rejects_symlink_without_mutating_target` | [event_stream.rs](../../../crates/dag-runtime/tests/run_loop/event_stream.rs) |
| 显式物理节点保留 | `explicit_physical_node_is_retained_by_child_dispatch` | [placement_tests.rs](../../../crates/control/src/transport/layered_tests/placement_tests.rs) |
| 未指定时独立选点 | `unspecified_physical_node_keeps_independent_child_scheduling` | [placement_tests.rs](../../../crates/control/src/transport/layered_tests/placement_tests.rs) |
| Codex 配对与真实工具执行 | 两节点独立 runc 模型工具探针 | [install-codex.sh](../../../scripts/dag-rootfs/install-codex.sh) |

- 最终 `cargo test --workspace`：447 个 harness 汇总 5786 passed / 0 failed / 8 原有 ignored；随后 docs 编译遇到临时缺失的 SPA 构建文件，原退出码 1 保留。标准 `npm run build` 在 artifacts 副本内成功后，仅补跑 `cargo test --workspace --doc`：21 个目标编译通过、0 个可执行示例、退出码 0；不重复已通过测试。原始日志位于 `/data00/workspace/artifacts/test-agent-capabilities/core/runtime-repair/evidence/` 下的 `event-final-test.log`、`final-doc-spa-build.log`、`event-final-doc-test.log`，各有同名 `.exit`。
- 最终 `cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace` 成功。证据为同目录的 `event-final-clippy.log` 和 `event-final-build.log`。私有匹配格式 6 包的事件、runc、调度定向回归共 7 项通过，与 main 的格式 7 构建隔离。
- 核心与 Python 修复阶段曾完成全仓 5775 passed / 0 failed / 8 ignored，日志 `test-workspace-complete.log`、退出码 0；不以该旧阶段结果替代最终扩展修改的检查。
- 核心组件单测 61 passed，真实 runc 两项通过。原源码在相同镜像下重现 PTY 与知识资源两个失败，保留原始日志及非零退出码。

## 相关

- [运行时索引](../../../agents/dag-runtime/index.md)、[DAG 功能](../../dag/index.md)、[Control](../../../agents/control/index.md)、[Brain](../../brain/index.md)。
