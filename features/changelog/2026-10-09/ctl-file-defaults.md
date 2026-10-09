# ctl 文件连接配置与身份结构迁移

ctl 自动读取 `$XDG_CONFIG_HOME/opencoder/ctl.json`，未设置 XDG 时读取 `$HOME/.config/opencoder/ctl.json`。文件支持 `server`、`token` 或 `token_file` 和 `verbose`，也可用 `--client-config` 指定文件。相对 Token 文件路径基于配置文件所在目录解析；命令行优先于环境变量，环境变量优先于文件。已有 admin Token 可以直接用于所有具有管理员权限的 API 请求。

存储 schema v35 将用户身份和 Token 分表；迁移保留已有凭据摘要、创建时间与有效性，旧只读角色转为 viewer。发布数据格式为 6，与旧格式必须通过维护切换升级。正式发布包和 Runtime 镜像包含 DAG、Agent 步骤和 Agent 会话三个运行器。维护窗口内恢复旧版本时，仅接受与迁移结果完全一致的身份与 Token；保留每个凭据摘要、创建时间和旧角色后恢复原表结构，出现新增、变更、过期或撤销记录则拒绝恢复。

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 配置读取、相对凭据路径、参数优先级 | `connection::tests` | `crates/ctl/src/connection.rs` |
| 无鉴权参数使用文件中的 admin Token 连接真实 Server，显式 Token 覆盖文件 | `default_file_connects_with_existing_admin_token_without_auth_arguments` | `crates/ctl/tests/server_local.rs` |
| 结构迁移保留所有旧凭据且不设置过期或撤销状态 | `admin_credential_is_preserved_when_users_and_tokens_are_separated` | `crates/store/tests/access_tokens.rs` |
| 数据契约禁止旧格式同时写入 | `identity_schema_requires_maintenance_from_previous_formats` | `crates/core/src/fleet/release.rs` |
| 发布工具接受新格式并拒绝新旧格式重叠 | `test_identity_format_verifies_but_cannot_overlap_legacy_identity_servers` | `scripts/platform/rolling_tests/test_manifest.py` |
| 镜像固定全部运行器且不改写源镜像 | `test_frozen_image_preserves_internal_hardlinks_without_linking_the_source` | `scripts/platform/rolling_tests/test_native.py` |
| 写入重开前恢复原身份结构，所有 Token 保持原值；变更凭据拒绝恢复 | `test_unopened_identity_migration_restores_old_schema_without_changing_any_token`、`test_identity_restore_refuses_any_new_or_changed_credential_or_identity` | `scripts/platform/maintenance_tests/test_archive.py` |
