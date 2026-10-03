Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 可选 Cua computer-use CLI

新增 `opencoder-computer` Python 包，通过已有 OpenCoder `/cli` 注册使用说明，由 Cua Agent 使用独立配置的桌面模型操作已有远程桌面。平台能力沿用实际 Cua 后端，Linux 不再限定 Wayland。调研、安装和远端准备见 [使用文档](../../../tools/computer-use/README.md)。

提供 `doctor/run/status/cancel`、动作上限、超时和信号取消；结果、事件及 PNG 截图留在运行目录。固定 Cua 源码提交，适配其系统信息、失败回执和连接关闭行为，保留服务端拒绝及明确任务失败。核心运行时不增加 computer-use 工具、数据库表或配置环境变量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 三种目标系统、模型与远端桌面循环 | `test_real_cua_loop_connects_model_to_remote_desktop` | [test_sdk.py](../../../tools/computer-use/tests/test_sdk.py) |
| 桌面检查、截图校验、超时与连接关闭 | `test_doctor_reports_real_sdk_versions_and_remote_screen`、`test_doctor_rejects_corrupt_screenshots`、`test_doctor_timeout_has_a_readable_error`、`test_real_sdk_websocket_fallback_keeps_headers_and_closes`、`test_backend_close_waits_until_client_socket_is_closed` | [test_sdk.py](../../../tools/computer-use/tests/test_sdk.py) |
| 原生能力拒绝及明确失败回执 | `test_native_backend_refusal_cannot_report_completed`、`test_native_computer_output_failure_is_not_success` | [SDK 测试](../../../tools/computer-use/tests/test_sdk.py)、[结果测试](../../../tools/computer-use/tests/test_results_state.py) |
| 动作上限、超时、取消与互斥 | `test_action_limit_stops_before_next_remote_action`、`test_timeout_closes_connections_and_releases_target`、`test_cancel_during_model_wait_stops_and_persists`、`test_concurrent_runs_cannot_control_same_target` | [test_sdk.py](../../../tools/computer-use/tests/test_sdk.py) |
| 命令分发、状态读取、进程取消与退出码 | `test_cli_real_sdk_run_and_doctor_dispatch`、`test_status_and_cancel_work_without_config_or_model`、`test_cancel_cli_stops_running_process`、`test_sigterm_persists_cancelled_result` | [test_cli.py](../../../tools/computer-use/tests/test_cli.py) |
| 凭证文件、输出遮盖与截图产物 | `test_config_reads_credentials_relative_to_config`、`test_diagnostics_preserve_json_stdout_and_redact_split_secrets`、`test_authentication_errors_redact_tokens_without_the_header_scheme`、`test_artifacts_cancellation_and_interrupted_status` | [配置测试](../../../tools/computer-use/tests/test_config.py)、[产物测试](../../../tools/computer-use/tests/test_results_state.py) |
| OpenCoder 注册加载与禁用、注入范围 | `computer_cli_example_loads_and_injects_into_primary_agent`、`disabled_computer_cli_is_not_injected` | [computer_cli_registration.rs](../../../crates/core/tests/computer_cli_registration.rs) |

实际输出：

```text
pytest tools/computer-use/tests -q
50 passed in 66.76s (0:01:06)

ruff check tools/computer-use
All checks passed!

cargo test -p opencoder-core --test computer_cli_registration
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

python -m build --wheel --outdir /tmp/opencoder-computer-wheel tools/computer-use
Successfully built opencoder_computer-0.1.0-py3-none-any.whl
```

安装包入口以及无 Cua SDK 环境下的结果查询已验证。自动化服务边界为本地 HTTP/WebSocket 桩；未连接真实 Windows/macOS/Linux 桌面或真实模型，不能把这些测试作为真机验收。

## 仓库回归

开始基线 `cargo test --workspace` 在已有工作区改动中编译失败：session 缺失 `powershell`/`host` 模块及相关符号。初次 `cargo clippy --workspace --all-targets -- -D warnings` 在已有 DAG runtime 改动中失败，涉及未使用导入及缺失 `extract_tail_bare_json`。这些区域没有在本功能中修改。

收尾时先通过 `cargo build --workspace` 更新测试依赖的可执行文件；随后 `cargo clippy --workspace --all-targets -- -D warnings` 通过。首次测试所用旧可执行文件及默认测试 rootfs 中的 runner 版本与当前节点不一致。后续回归使用临时复制的测试 rootfs 和当前构建的 runner，通过现有 `DAG_TEST_ROOTFS` 仅指定该测试进程的镜像；未修改默认镜像或 DAG 版本检查。

```text
cargo build --workspace
Finished `dev` profile [unoptimized + debuginfo] target(s) in 24m 06s

cargo clippy --workspace --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 13m 15s
```

更新测试镜像后的并发回归在 `dag_e2e` 停止，该目标 6 项通过、4 项失败，全部为等待状态超时；本功能未修改这些测试或对应实现。实际输出：

```text
cargo test --workspace
timed out after 30s waiting for spin step running
timed out after 180s waiting for terminal status for dag-e2e-structured-1
timed out after 180s waiting for terminal status for dag-dynamic-runc
timed out after 180s waiting for terminal status for dag-pool-run-2
test result: FAILED. 6 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 341.73s
error: test failed, to rerun pass `-p opencoder --test dag_e2e`
```

最终全量复跑前，重新构建正式可执行文件，并核对 Server、Agent 和临时 rootfs 中两个受检 runner 的完整构建信息一致，提交均为 `7687b5f5`。之前完整收集到的 106 项失败均由旧 Server 或旧测试镜像的构建信息引起。

最终 gate 通过：解析 stdout 中 446 个测试目标的结果，合计 **5639 passed / 0 failed / 7 ignored**；7 项为已有手动验收用例，本功能没有新增跳过。构建并行设为 8、测试并行设为 2；测试镜像仅通过测试进程的 `DAG_TEST_ROOTFS` 指定。

实际输出摘录：

```text
cargo build --workspace --bins
Finished `dev` profile [unoptimized + debuginfo] target(s) in 44.03s

cargo clippy --workspace --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 47.96s

cargo test --workspace --no-fail-fast -- --test-threads=2
Finished `test` profile [unoptimized + debuginfo] target(s) in 10m 42s

Running tests/dag_e2e/main.rs
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 70.79s

Running tests/computer_cli_registration.rs
test disabled_computer_cli_is_not_injected ... ok
test computer_cli_example_loads_and_injects_into_primary_agent ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## 相关索引

- [computer-use 实现](../../../agents/computer-use/index.md)
- [computer-use 能力](../../computer-use/index.md)
