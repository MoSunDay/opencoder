Commit: 823be6337bee6ab221840d61f5baf632b3e8f9d4 8a25e3eb4e50008d0a104ab3c33ff24f3e246bd6 4ec0637e71be1b14d82399c9d5d9fc9aabce860f
# Operator 执行的 Linux 专属账号与降权边界

Linux 节点上 Operator 执行原先与节点进程同用户运行，仅有 HOME/WORKSPACE 目录隔离；任何 operator 子进程都可写节点文件。现在每条 Operator 执行供给一个专用系统账号（`ocop-` 前缀、nologin shell），形成 OS 级读写边界，Windows 路径保持原有环境隔离不变。

## 变更及边界

- 新增 `crates/worker` 身份供给模块：`useradd -r -M` 建号、home/workspace 归属该账号 0700、冻结配置快照改为 root 属主 0440（会话无法改写自身配置）；执行终态把产物 chown 回节点用户并 `userdel`。
- 准入 fail-closed：非 root 或缺少 shadow-utils 时拒绝 Operator 执行，不静默降级；operator 配置 `linux_identity: off` 作为本地开发逃生阀。配置合并链路同步识别该字段。
- 身份随 HarnessRuntime 的 `run_as` 持久化（serde 向后兼容旧记录），节点重启恢复会话后继续生效；fork 为新执行供给全新账号，不复用父身份。
- 隐藏 supervisor 协议扩展 `--uid/--gid/--groups`：工作负载 exec 前按 setgroups→setgid→setuid 固定顺序降权并收紧 umask，拒绝 uid/gid 0 的降权请求；lease/PDEATHSIG/subreaper 语义不变。DAG runc 链路不传身份，规则 04 边界不变。
- macOS fallback 与 Windows 进程后端接受并忽略身份参数，TUI/本地路径零改动；环境变量不承载身份，全部经结构体传递。
- 已知边界：降权仅在节点配置隐藏 supervisor 时生效（生产 agent 二进制始终配置，库内嵌会话为直连子进程）；队列路径终态依赖供给前补扫回收账号。
- Server 二进制同步改为 Linux-only 运行时门控（其他平台启动即报错，保持可编译，CLI/TUI 跨平台不受影响）。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 建号、目录归属与快照冻结 | `provision_creates_the_account_and_materialize_moves_ownership` |
| 终态回收产物并删除账号 | `release_hands_files_back_and_deletes_the_account` |
| fork 供给独立新账号 | `fork_provisions_a_fresh_distinct_account` |
| off 逃生阀保持节点属主并放行 | `off_mode_keeps_the_node_owned_layout_and_admits_any_node` |
| run_as 持久化与旧记录兼容 | `run_as_round_trips_and_legacy_runtimes_deserialize_without_it` |
| supervisor 降权（id -u/-g、umask、越权写失败） | `supervisor_runs_the_workload_as_the_provisioned_user` |
| 无身份请求保持节点用户 | `supervisor_without_identity_keeps_the_node_user` |
| Server 平台门控 | `platform_gate_is_deterministic_and_message_names_linux_and_tui` |

## 验证范围

- `cargo test -p opencoder-worker --test operator_linux_identity`：5 用例通过（root 实跑 useradd/userdel，测试后 `/etc/passwd` 无 `ocop-` 残留）。
- `cargo test -p opencoder-session --test supervisor_run_as_linux`：2 用例通过（真实降权断言 uid/gid/umask 与越权写 EACCES）。
- `cargo test -p opencoder-core --lib harness:: platform::`、`cargo test -p opencoder-worker --lib operator`、`cargo test -p opencoder-server --bin opencode-server`：通过。
- `cargo clippy -p opencoder-worker -p opencoder-core -p opencoder-session --all-targets -- -D warnings`：零警告。
