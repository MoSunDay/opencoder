Commit: f23063530a72e73bddef57a2d077c39754b16e29

# TODO 关键信息与独立过程文件存储

TODO 正文显示为“关键信息”，保留 `draft` 数据和 Markdown 编辑方式，便于持续同步与交接。
新增文件页签及 CLI 上传、列表、单文件下载和删除，用户主动保存的资料由 Server 持有，
专用 NFS 只读共享。目录固定为 TODO 创建时间的 UTC 日期与 TODO ID；原生执行过程继续留在节点。

文件完整上传后才发布，同名不覆盖，大小默认上限 512 MiB。路径越界、符号链接和特殊文件被拒绝。
有文件或存储不可用时禁止删除 TODO；文件发布与任务删除使用跨进程锁。
关键信息、评论和文件分别提交，切换页签保留草稿，context 提供文件清单及下载入口。
文件、评论和执行操作分别记录忙碌状态，任何一项未完成时都保留关闭保护。

部署工具补齐第五路 NFS 的状态与端口检查、持久化根和协调目录、专用 TODO 服务管理代理、
文件上传路由的 Nginx 限制、备份容量估算及实际文件根的备份恢复。已有根目录通过绑定文件固定，
配置变化或目录丢失返回错误。独立启动的 NFS 默认关闭；受管部署通过兼容版本启用专用服务，已有鉴权数据保持不变。

## 功能与测试

| 功能 | 测试名与入口 |
|---|---|
| 配置默认值、嵌套合并、非法配置拒绝 | `defaults_and_nested_overlays_preserve_storage`、`invalid_storage_configuration_is_not_silently_ignored`，core config/project |
| 日期目录、原正文兼容、Server 数据目录迁移、重启后下载与删除保护 | `files_roundtrip_old_todo_date_restart_and_delete_guard`，control `project_files` |
| 权限、限额、断连、越界与符号链接 | `invalid_paths_size_interruptions_and_permissions_do_not_publish_files`、`symlinks_special_files_and_config_rebinding_are_rejected` |
| 跨进程上传与删除、同名竞争 | `concurrent_uploads_and_deletion_share_a_filesystem_lock` |
| 绑定目录丢失不能视为空列表 | `missing_bound_storage_is_an_error_instead_of_an_empty_list` |
| 原有默认数据目录与 TODO 删除流程 | `goals_initiatives_todos_crud_roundtrip`、`plan_after_todo_delete_resolves_404`，control `e2e` 定向用例 |
| 独立资源服务代理、真实 NFS 读取与写入拒绝 | `dedicated_service_exports_uploaded_bytes_and_rejects_nfs_writes` |
| CLI 二进制传输、清单与交接上下文 | `cli_upload_download_list_delete_and_handoff_context`、`parses_and_routes_binary_file_commands` |
| 文件清单缺失明确报告 | `context_reports_unavailable_task_files_without_claiming_complete_handoff`，CLI `project_context` |
| 页面上传、下载、确认删除、权限与失败保留 | `project/files/panel.dom.test.jsx`、`project/files/api.test.js`、`api.test.js` |
| 改名及现有表单、页签、执行、草稿与并发操作关闭保护 | `todoTabs.dom.test.jsx`、`editor.dom.test.jsx`、`todosTab.dom.test.jsx`、`editing.dom.test.jsx`；NFS 卡片直接影响 `agentsConfig.dom.test.jsx` |
| 自定义根备份恢复、根绑定、端口与代理限制 | `rolling_tests/test_todo_files.py`；直接影响的 storage、preflight、capacity、probes 及部署代理用例 |
| 真实 Server、Nginx、浏览器与四种屏宽 | [project_files_ui.js](../../../scripts/acceptance/project_files_ui.js) |

## 首轮本地验证

- `cargo test -p opencoder-core --lib config::project`：2 passed。
- `cargo test -p opencoder-control --test project_files`：6 passed。
- control `e2e` 仅运行上述两个 TODO 删除相关用例，2 passed；发现并修复默认基础数据目录经符号链接迁移到其他磁盘时的兼容问题，专用存储子目录仍拒绝符号链接。
- `cargo test -p opencoder-cli --test project_files`：1 passed；`--test project_context`：4 passed；CLI 文件命令解析定向用例：1 passed。
- SPA 定向测试覆盖上述 8 个文件，共 45 个用例通过；未运行 SPA 全量测试。
- Python 定向执行：`rolling_tests/test_todo_files.py` 3 passed、`rolling_tests/test_ontology.py` 2 passed、`storage_tests/test_storage.py` 10 passed；维护 preflight 与 rolling probes 共 21 passed，capacity 4 passed，既有二进制上传代理限制用例 1 passed。曾发现并修复在线备份缺少目标父目录，修复后对应测试通过。
- `cargo clippy -p opencoder-core -p opencoder-control -p opencoder-cli -p opencoder-web --all-targets -- -D warnings`：通过。
- SPA 构建、Server/CLI 构建通过。浏览器使用真实隔离 Server 和生成的 Nginx 配置完成 3 MiB 二进制上传下载、同名冲突、任务删除保护及标题/评论草稿保留；1920/1280/768/390 四种屏宽无外层横向溢出，页面错误为 0。
- 浏览器验收命令为 `node scripts/acceptance/project_files_ui.js <本次构建的Server> crates/web/spa/dist <新证据目录>`；本次回执 `/tmp/opencoder-todo-files-ui-20261016h/receipt.json`，SPA SHA256 为 `9470d8845e643f012e7aa54d8a01e0c1ab5da913d7114073222105bc856701f1`。
- 首轮仅覆盖本次改动及直接影响链路，未执行全量测试；上线以独立发布回执为准。

相关：[任务文件用法](../../../docs/project-files.md)、[项目工作台](../../project/index.md)、[项目约定](../../../rules/07-project-module-contract.md)。

## 独立托管与发布保护

- TODO NFS 使用专用进程，回环管理接口与现有资源服务分离；异常返回 503，不回退到旧服务。
- 首次安装保存可恢复记录，固定服务二进制并检查运行进程摘要；重复发布只核验，不重启导出。
- 先发布兼容版本，再开放上传；最低回滚能力要求在配置变更前持久化，禁止退回缺少删除保护的 Server。
- 新增验证映射：独立 NFS/代理 `dedicated_service_exports_uploaded_bytes_and_rejects_nfs_writes`；关闭上传 `disabled_uploads_preserve_reads_and_delete_protection`；无回退 `unavailable_dedicated_owner_never_falls_back_to_local_export`；无数据库认证 `private_seed_authentication_needs_no_database`；安装恢复与回滚 `rolling_tests/test_todo_service.py`。

后续定向验证：独立托管后的 control `project_files` 8 passed；core 配置 2 passed；Web 鉴权 6 passed；Server 参数 4 passed；相关五个 crate 的 Clippy 零警告。Python 存储与安装恢复 9 passed、部署 29 passed、信号 10 passed、manifest 15 passed、probes 14 passed、CLI 3 passed、维护 flow 11 passed。未运行全仓测试。

公开入口验收由 `scripts/acceptance/todo_files/live.py` 和 `browser.js` 覆盖两个任务、五种文件、HTTP/CLI/内核 NFS 内容校验、四种屏宽、静态产物一致性及 900 秒持续观察。发布连续性继续使用现有 `smooth_release/live.py`。

恢复首次服务安装前核验当前 Server 已不再依赖 TODO 存储，避免撤销仍在使用的服务；用例 `test_restore_cannot_remove_storage_used_by_the_active_server` 覆盖拒绝发生在任何系统操作之前。
