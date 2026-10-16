Commit: bd88531e3f3c1ddea3b9eb4228b23532009c38ad

# 执行设置的共享目录配置

「执行设置」通过能力列表读取 TODO 模板。Server 缺少 `agent.share_dir` 时回退到服务 HOME；线上 HOME 绑定到已经删除的目录，能力列表及环境、工具、模板接口因此返回 500。

部署准备现在将共享目录固定在持久化根下的 `server/.opencoder/share`，设置 Server 用户权限并检查路径冲突。Server 路径不会继承到新建的执行节点配置。

线上修复仅补齐现有 Server 角色配置，配置备份保存在主机的 `backups/share-root-fix-20261012/`。已有共享目录已核验，应用进程无需重启；没有修改鉴权数据。

## 验证

- 目录配置、创建和重复准备：`test_one_root_drives_configs_mounts_runtime_and_server_units`。
- 已有其他共享目录拒绝隐式覆盖：`test_conflicting_managed_paths_and_writable_exports_fail_before_preparation`。
- 严格 umask 下实际服务用户可创建和删除文件：`test_restrictive_umask_and_existing_root_allow_the_actual_service_user`。
- `scripts/platform/storage_tests` 的 12 项定向测试通过。
- 线上能力列表和环境、工具、模板接口返回 200；临时环境创建、读取、更新、删除通过，能力列表返回 99 项。
- 公共入口的 TODO 编辑页面无共享目录报错，「执行能力」下拉选项正常展示；页面没有 JavaScript 异常。

相关索引：[Server](../../../agents/server/index.md)、[发布配置](../../../docs/smooth-release.md)。
