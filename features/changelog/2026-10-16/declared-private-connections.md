Commit: 93acdeaa53cd864e85760a2123efb20526d92051

# 启动配置显式声明额外连接

OpenCoder 的容器启动仅提供通用连接文件挂载。启动配置通过可选 `connection_files` 数组声明 JSON 文件的绝对路径，运行时按声明读取文件及其 `token_file`，不按外部工具名称推断路径。测试样例和说明使用通用资源、连接与任务名称。

平台 CLI 连接保持必需；额外声明缺失、格式错误、路径非绝对或包含软链接时拒绝启动。挂载只读，配置和挂载清单随运行冻结。省略或传空数组时不增加连接；需要额外连接的既有配置须在下一次启动前补充声明。已保存运行不改写挂载或凭据。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 文件与 token 按声明挂载、配置冻结 | `managed_startup_freezes_config_and_mounts_only_declared_private_files` | [startup_tests.rs](../../../crates/dag-runtime/src/sandbox/codex/startup_tests.rs) |
| 不扫描未声明连接、空数组和省略字段 | `managed_startup_only_reads_explicit_connection_files` | 同上 |
| 非法路径、软链接、缺失文件与 token 拒绝 | `managed_startup_rejects_invalid_connection_declarations_and_missing_tokens` | 同上 |
| 真实 runc 中只读挂载、未声明文件不可见、源文件不变与容器回收 | `connection-mount-probe` | [example](../../../crates/dag-runtime/examples/connection-mount-probe.rs) |
| 通用长路径继续覆盖共享 NFS 句柄 | `acl_clone_resolves_backend_handles_without_scanning_the_export` | [nfs_shared_handles.rs](../../../crates/agents/tests/nfs_shared_handles.rs) |
| 多个资源提供方隔离、版本固定及恢复 | `named_authorities_are_isolated_immutable_and_frozen_across_restart` | [resource_admission.rs](../../../crates/control/tests/resource_admission.rs) |
| 通用输入副本名称继续覆盖备份边界 | `BackupScopeTests` | [test_snapshot.py](../../../scripts/platform/rolling_tests/backup/test_snapshot.py) |

## 实际验证

- `cargo test -p opencoder-dag-runtime --lib sandbox::codex::startup::tests -- --nocapture`：4 passed。
- `cargo test -p opencoder-agents --test nfs_shared_handles`：1 passed。
- `cargo test -p opencoder-control --test resource_admission`：3 passed。
- `python3 -m unittest discover -s scripts/platform/rolling_tests/backup -p test_snapshot.py -v`：8 passed。
- `cargo run -p opencoder-dag-runtime --example connection-mount-probe`：真实 runc 验证通过，6 个私有文件挂载保持只读，未声明文件不可见，源文件未变化，剩余容器为 0。探针前两次分别因自身缺少 `/proc` 挂载、空列表返回 `null` 的解析问题失败；修正探针后通过。
- `cargo clippy -p opencoder-dag-runtime -p opencoder-agents -p opencoder-control --all-targets -- -D warnings`：通过，零警告。修改的 Rust 文件格式、差异空白、文档链接和新增代码文件行数检查通过。
- 测试范围仅为连接挂载及本次调整样例直接涉及的 NFS、资源准入和备份链路；未运行全量测试，未部署线上版本。使用临时配置、占位 token 与临时数据库。

相关：[运行时结构](../../../agents/dag-runtime/index.md)、[DAG 能力](../../dag/index.md)、[配置说明](../../../docs/registered-runners.md)。
