Commit: (working-tree, 基于 b29cc9776082f7da8d089e6b4ae43c4ea43bc524)

# 完整资源分发、节点快照与发布验收

Server NFS 不再将 60 字节以上的资源路径当作不存在。短路径句柄兼容旧客户端，长路径使用有界摘要并支持导出重启恢复；目录读取失败明确返回错误。Node 显式资源路径消失时拒绝创建快照，失败复制不发布半成品；已接受的任务继续使用节点本地快照。任务过程、数据库和产物仍仅由所属节点持久化，Server 保持五字段执行索引。

关联 Select 按记录隔离待保存状态，切换 TODO 时旧请求不能覆盖新记录。此前发布的 Markdown 保留及独立专项模型保持不变。

验收工具统一验证完整平台 bundle，分别记录平台执行、业务结论和整体结果；依赖准备失败、未执行断言、证据不完整均不能签收成功。新增使用真实业务 API、Runner 和 Codex 的完整输入正例；历史业务失败保持原样。修正旧浏览器入口的导航、状态文案、消息分页和 Wasm 接口，取消固定时长观察，保留历史与数据完整性校验。业务验收结束只停止自建服务和挂载，保留数据库与证据。

新增本机 systemd 只读 NFS 挂载模板和 Agent 挂载依赖，使用有界失败及无属性缓存配置。回滚旧 Server 时需要停止依赖节点并重新挂载，不能仅替换二进制。

## 测试覆盖

| 功能 | 测试或入口 |
| --- | --- |
| 深层路径、旧句柄、重启与分页 | `complete_skill_package_survives_long_paths_and_export_restart`、`long_directory_pagination_and_old_handles_survive_restart` |
| 目录读取失败不能伪装完整结果 | `directory_errors_cannot_become_a_successful_partial_listing` |
| 资源源路径丢失、复制失败和重试 | `absent_configured_source_is_not_an_empty_successful_snapshot`、`failed_copy_publishes_nothing_and_retry_uses_complete_resources` |
| 内核挂载、两节点快照及离线继续 | `readonly_nfs_node_snapshots_and_offline_followup` |
| 切换记录期间关联保存失败 | `isolates pending association and late save failure when the record changes` |
| bundle 与质量门禁 | `scripts/acceptance/business/tests/test_release.py` |
| 项目历史、分页、取消及不可变产物 | `scripts/acceptance/project/main.js` |
| 真实 Runner 与业务结果 | `scripts/acceptance/business/main.py --scenario positive` |

候选验证与发布证据归档于本次独立验收目录，最终发布回执记录实际提交、运行文件摘要、E2E 和当前健康结果。
