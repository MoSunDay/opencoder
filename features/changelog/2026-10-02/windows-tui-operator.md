Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# Windows 原生 TUI 与 Operator

## 变更

- 增加 Windows 11 x64 原生 TUI 与 Operator 节点，需要 PowerShell 7.4 及以上的 7.x 稳定版；Linux/macOS 保留 Bash。
- 命令、hooks 与本地 TUI 命令共用宿主执行入口。Windows 使用 Job Object 管理进程树；只读模式解析 PowerShell AST，不执行被检查命令。
- 私有文件在创建时配置 Windows ACL，原子替换与不可变发布共用平台入口；Operator 隔离 Windows 用户目录变量。磁盘准入不伪造 inode 数量。
- Windows 仅声明并接受 Operator；DAG 容器、NFS、其他执行种类及原生 Server 不进入支持范围。Linux 单节点单容器、只读 NFS 与本地写层契约保持原有约束。
- 增加原生 Codex 进程与线程恢复测试、ZIP 打包、成组升级与失败恢复的 PowerShell 安装器，以及 Windows MSVC CI。
- 回归修正服务端重启测试的时序条件：暂停前两个并行子任务，在服务端离线时检查二者仍运行，再放行完成与恢复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 宿主工具与提示一致 | `host_tool_and_prompt_agree` | [core 平台 shell](../../../crates/core/src/platform/shell.rs) |
| 私有写入、原子替换、并发不可变发布 | `private_files_replace_atomically_and_publish_once_under_concurrent_writers` | [platform_fs.rs](../../../crates/core/tests/platform_fs.rs) |
| Windows ACL、junction 与长路径 | `windows_acl_rejects_public_access_and_junctions_and_supports_long_paths` | [platform_fs.rs](../../../crates/core/tests/platform_fs.rs) |
| PowerShell 名称、Unicode、输出及退出码 | `native_shell_handles_unicode_paths_streams_exit_and_tool_registration` | [windows_native.rs](../../../crates/session/tests/windows_native.rs) |
| AST 检查不执行命令 | `powershell_ast_gate_blocks_writes_without_evaluating_them` | [windows_native.rs](../../../crates/session/tests/windows_native.rs) |
| 取消、自然结束及父进程强制退出清理 | `cancelling_shell_and_natural_exit_stop_all_descendants`、`killing_owner_process_closes_job_and_kills_grandchildren` | [windows_native.rs](../../../crates/session/tests/windows_native.rs) |
| 原生 Codex 流式输出与线程恢复 | `native_codex_streams_and_resumes_the_persisted_thread` | [windows_native.rs](../../../crates/session/tests/windows_native.rs) |
| junction 搜索去重 | `native_search_visits_junction_targets_once` | [windows_native.rs](../../../crates/session/tests/windows_native.rs) |
| 后台超时与输出溢出清理 | `powershell_timeout_handoff_completes_and_output_overflow_stops_background` | [windows_tests.rs](../../../crates/session/src/tools/command/windows_tests.rs) |
| Windows Operator 准入、隔离、执行与恢复 | `windows_operator_rejects_other_workloads_executes_and_recovers_isolated_home` | [windows_operator.rs](../../../crates/worker/tests/windows_operator.rs) |
| TUI 宿主命令与超时清理 | `windows_tui_short_commands_use_powershell_and_clean_timeout` | [windows_tui.rs](../../../tests/windows_tui.rs) |
| 自动记忆会话的宿主工具列表 | `enabled_memory_uses_a_context_copy_after_main_completion` | [local_memory/mod.rs](../../../crates/session/src/runner/local_memory/mod.rs) |
| 服务端重启不停止并行子任务 | `server_restart_preserves_running_parallel_children_and_resumes_brain` | [brain_server_restart.rs](../../../crates/worker/tests/brain_server_restart.rs) |
| 安装、升级、校验失败与锁定 EXE | PowerShell 安装器验收 | [installer-tests.ps1](../../../scripts/platform/windows/installer-tests.ps1) |

## 安装恢复与只读执行补充

- Windows 最低版本明确为 PowerShell 7.4 及以上的 7.x 稳定版。只读命令从 AST 的源位置生成受控调用，保留 UTF-16 位置、中文、引号和管道；Plan 与 Sidecar 实际执行生成后的命令。
- Git 查询清除继承的 Git 环境覆盖，禁用 pager、外部 diff、textconv、fsmonitor、签名检查和自动获取对象；拒绝内容过滤器、部分克隆及未确认的参数。Git for Windows 的空配置使用 `/dev/null`。rg 忽略外部配置，拒绝预处理与压缩解码程序。
- 安装器增加私有、不可覆盖的原始恢复记录、持久化进度、独占锁和显式 `-Recover` / `-Rollback`。两个 EXE 通过整个目录切换；Windows 文件校验句柄在目录改名前关闭，安装互斥锁持续持有。恢复可在再次中断后重试，只撤销本次拥有的 PATH 项，保留其他项及空段。
- Win32 文件入口支持长路径；包内校验和覆盖安装器的两个模块。

### 补充测试覆盖

| 功能 | 测试名 / 用例 | 文件 |
|------|----------------|------|
| 稳定版 PowerShell 最低版本 | `native_argument_policy_requires_a_stable_supported_powershell` | [host.rs](../../../crates/session/src/tools/command/host.rs) |
| Git/rg 参数限制及 UTF-16、引号、管道 | `query_options_do_not_admit_execution_or_output_overrides`、`prepared_commands_preserve_utf16_spans_pipelines_and_literal_arguments` | [readonly/tests.rs](../../../crates/session/src/tools/command/readonly/tests.rs) |
| 外部配置及实际 Plan/Sidecar 调度 | `prepared_read_only_commands_disable_git_and_rg_external_configuration` | [windows_native/readonly.rs](../../../crates/session/tests/windows_native/readonly.rs) |
| 安装、升级、包错误、锁定 EXE、并发、PATH 恢复 | PowerShell 安装器功能验收 | [installer-tests.ps1](../../../scripts/platform/windows/installer-tests.ps1) |
| 不可变恢复依据、长路径及五处真正进程中断 | `prepared`、`old_moved`、`new_moved`、`discarded`、`old_restored` | [installer-state-tests.ps1](../../../scripts/platform/windows/installer-state-tests.ps1) |

## 原生实机发现与修复

- Windows 用户目录按 USERPROFILE / APPDATA / LOCALAPPDATA 解析，Operator 的四个用户目录变量随任务隔离。TUI 可使用配置快照中的原生 Codex 路径与代理；加载磁盘配置时校验私有运行参数，错误不带凭据值。
- TUI 的 MSVC 程序预留 8 MiB 栈，进程监督入口在创建异步主流程前执行；工作目录发布先关闭已同步的文件句柄，避免 Windows 目录改名失败。
- libsql 发布版 0.9.30 在连接释放时重复关闭 SQLite 句柄，实机出现访问异常。固定使用上游 PR #2282 的提交 `0070ff3331cd6d09425b812e1cd3ebe32e1d4206`，补充最后持有者与文件释放回归；不引入私有第三方代码副本。
- 仓库逻辑索引从 `agents.md` 改名为 `repo-memory.md`，消除与指令文件 `AGENTS.md` 的 Windows 大小写冲突；原有索引内容与引用保留。
- Linux mount unit 源文件使用可移植的模板名称，部署仍通过 systemd-escape 生成准确名称；UI 的 NFS 测试端口避开内核临时端口区间。

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 磁盘私有 Codex 与运行配置、错误脱敏 | `disk_config_preserves_codex_launch_settings_and_registered_profiles`、`malformed_private_harness_settings_fail_without_exposing_values` | [config/agent.rs](../../../crates/core/src/config/runtime/agent.rs) |
| Windows 程序主入口与构建身份 | `windows_main_starts_and_reports_product_metadata` | [windows_tui.rs](../../../tests/windows_tui.rs) |
| SQLite 关闭、派生持有者与数据库文件释放 | `embedded_connections_close_safely_across_fresh_runtimes_and_last_owners`、`last_store_connection_owner_releases_the_native_database_file` | [connection_lifecycle.rs](../../../crates/store/tests/connection_lifecycle.rs) |

- macOS 的 Codex 错误流测试单独计时进程就绪与错误处理；用六秒启动延迟覆盖五秒错误处理预算，仍要求错误、缺失终态及无 Done 事件。测试：`codex_malformed_stream_and_missing_terminal_fail`（`crates/session/tests/harness_codex.rs`）。

## 当前验证

- 隔离候选的 Linux 全量回归：441 个测试目标，5,608 passed / 0 failed，7 个既有 ignored；全工作区 Clippy、构建、格式检查通过。计数来自 `candidate-fixed-workspace-tests.log`，不含共享工作区其他需求的测试。
- win-12（Windows 11 x64）原生 MSVC 构建、测试构建与六个支持包 Clippy 通过。core/session/TUI/worker 单元测试分别通过 309/440/1727/71 个；平台文件 2、会话与进程 7、Operator 1、TUI 2、数据库生命周期 2 个集成测试通过，连接关闭循环 5,000 次。
- 实机调试构建已完成真实模型流式回复、Codex 流式回复与同一会话恢复；Operator 取消与节点强制退出清理 PowerShell 子进程，重启保留中断状态并支持显式恢复。TUI 中文输入、模型菜单、窗口缩放、剪贴板图片与正常退出已执行。
- 正式发布包必须由干净提交经 [原生打包脚本](../../../scripts/platform/release/build-windows.ps1) 生成，安装器功能与五处进程中断恢复由 [平台 CI](../../../.github/workflows/platform.yml) 验证。调试构建与测试 EXE 的记录不能替代正式包验收。完整本轮证据存于 `/tmp/opencoder-windows-delivery-20261003`。

## 相关

- [Windows 支持范围](../../windows/index.md)、[安装与验收](../../../docs/windows.md)
- [core](../../../agents/core/index.md)、[session](../../../agents/session/index.md)、[TUI](../../../agents/tui/index.md)、[worker](../../../agents/worker/index.md)

- 子任务持续进展验收使用虚拟时间和六次模型增量，验证总时长超过空闲时限仍成功结束；取消和停滞验收继续检查实际存储状态。对应 `sustained_activity_does_not_timeout`、`timeout_marks_subagent_cancelled`、`stalled_single_step_times_out`。

- 原生 Codex 的重连 `error` 事件继续显示状态并允许重试和传输切换；`turn.failed`、非零退出、缺少完成事件仍失败关闭。对应 `reconnect_errors_leave_the_turn_open_until_success_or_failure`、`terminal_failure_stays_fatal`、`codex_reconnect_and_transport_fallback_finish_and_resume`、`codex_malformed_stream_and_missing_terminal_fail`。
