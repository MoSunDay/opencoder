Commit: 3c44c26640ff289d409d1ebf1c57ae1a17f33196

# 版本部署使用单一持久化根目录

`deployment.state_dir` 统一派生 Server、Host、Runtime、资源服务、工作目录、资源池、镜像、版本包、备份与受控启动器状态路径；移除重复的 `server_data`、`server_workdir`、`agent_workdir` 配置口。唯一 `deployment.json` 保存部署声明，Server/Agent 角色配置不保存 deployment 段。`--storage-plan` 预览布局，`--prepare-storage` 生成根内角色配置和只读 mount units，不启动服务、覆盖已有不同文件或自动迁移数据库。外部自定义启动器单独同步候选脚本，在空闲窗口迁移其锁与回执；凭据和既有 Codex SQLite 库保持原位。

外部 DAG 源工作区保持根外原路径。已有 Ontology 正文绑定不一致时拒绝准备，须先验证所有 ready revision 后显式迁移。没有新增环境变量或数据库表；不修改鉴权数据；仅 Python 部署控制器与生成配置改变，不要求重新构建 Rust 二进制。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 单根贯通配置、Server/Host unit、Runtime 冻结路径 | `test_one_root_drives_configs_mounts_runtime_and_server_units` | `scripts/platform/storage_tests/test_storage.py` |
| 真实 CLI 预览、生成和重复执行 | `test_preview_and_preparation_use_the_real_cli_without_starting_services` | `scripts/platform/storage_tests/test_cli.py` |
| 已有角色配置无须重复声明路径、保留独立模型 | `test_existing_layout_is_read_from_role_configs_without_repeating_paths` | `scripts/platform/storage_tests/test_storage.py` |
| 拒绝旧字段、可写导出、源目录重叠和目录别名 | `test_redundant_deployment_paths_require_explicit_migration` 等 | `scripts/platform/storage_tests/test_storage.py` |
| 只读检查 Ontology 绑定、拒绝静默迁移 | `test_ontology_binding_is_checked_read_only_before_preparing_new_paths` | `scripts/platform/storage_tests/test_storage.py` |
| umask 077/既有0700根下实际服务身份可读配置并执行程序 | `test_restrictive_umask_and_existing_root_allow_the_actual_service_user` | `scripts/platform/storage_tests/test_storage.py` |
| 资源服务没有对自身的 systemd 顺序依赖 | `test_resource_service_has_no_ordering_dependency_on_itself` | `scripts/platform/storage_tests/test_storage.py` |
| 真实 runc、双版本切换、回滚及历史归属保留 | `exercise` | `scripts/acceptance/storage_root/main.py` |

- 定向回归：`python3 -m unittest discover -s scripts/platform/storage_tests -v` → 12 passed。
- 配置签名直接影响链路：`rolling_tests` 68 passed、`maintenance_tests` 67 passed、`signal_tests` 12 passed；未运行仓库全量测试。
- 隔离验收：首次、第二版本与回滚后的真实 runc 探针均完成，执行目录均位于指定根；116 次公共就绪探测零失败，最大探测间隔/耗时 0.714 秒；源工作区 inode、内容、所有者与权限保持不变。证据：`/data00/workspace/artifacts/opencoder-storage-root-20261012/opencoder-smooth-n0gc3zln/storage-result.json`。
- 未改 Rust，Clippy 不适用。验收使用已有成套 `42456123489c6fbb5f108a575e249d48de59d5eb` 二进制，仅验证部署配置及其直接影响链路；远端服务恢复另行验收。

相关说明：[发布配置](../../../docs/smooth-release.md)、[平台行为](../../agent-platform/index.md)、[Server 逻辑](../../../agents/server/index.md)。

## 双节点发布准备

本节点发布基线为 `3c44c266`，远端基线为归档源码 `89a5599d`；逐项对照确认生产 Rust 代码及 SPA 相同，差异主要为测试和交付工具。本次保留本节点当前源码与代理启动器组织，仅引入本页记录的存储改造。补充验证 `smooth_release/tests` 15 项、`scripts/harness/test_codex_proxy.py` 3 项通过，SPA 重建无漂移；不重复执行未变化且已通过的定向测试。正式包、节点迁移、信号发布和 900 秒观察回执保存到 `/data00/opencoder-delivery/single-root-release/`。
