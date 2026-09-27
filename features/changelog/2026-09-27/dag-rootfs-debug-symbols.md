Commit: 720c3f03f784d857bbcac52fbf6bdc8992b2bcc6

# DAG rootfs 运行器体积

`prepare-dag-rootfs.sh` 安装三个 debug 示例程序时，仅从 rootfs 副本移除调试符号；构建目录原件保留。这样每个 runc 步骤复制私有 rootfs 时不再反复复制超过 1 GiB 的调试信息，避免节点执行尚未启动就耗尽 E2E 终态等待时间。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Agent 步骤在 runc 内完成 | `agent_step_session_runs_inside_runc_container` | `tests/dag_e2e/agent_runc.rs` |
| 动态 Agent 与 Wasm 步骤在 runc 内完成 | `runc_dynamic_agent_and_wasm_read_isolated_copies_and_argv` | `tests/dag_e2e/dynamic.rs` |

- 精简后专项回归：两项均通过，分别用时约 25 秒和 20 秒。
- 脚本语法：`bash -n scripts/prepare-dag-rootfs.sh` → 通过。
