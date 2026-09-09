Commit: 2a658f433024832a438e85ffd982c1b986f3b0fa

# 完整资源分发、节点快照与发布验收

Server NFS 不再将 60 字节以上的资源路径当作不存在。短路径句柄兼容旧客户端，长路径使用有界摘要并支持导出重启恢复；目录读取失败明确返回错误。Node 显式资源路径消失时拒绝创建快照，失败复制不发布半成品；已接受的任务继续使用节点本地快照。任务过程、数据库和产物仍仅由所属节点持久化，Server 保持五字段执行索引。

关联 Select 按记录隔离待保存状态，切换 TODO 时旧请求不能覆盖新记录。此前发布的 Markdown 保留及独立专项模型保持不变。

DAG 执行详情使用通用执行事件水位，修复误用会话 `/seq` 接口导致的 400。水位独立于分页，空执行为 0；详情就绪后才订阅，旧执行迟到响应不能覆盖新详情。

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
| DAG 空态、跨页水位与切换隔离 | `dag_replay_watermark_includes_events_beyond_the_requested_page`、`opens an empty DAG transcript through the generic execution API`、`ignores old execution frames after switching IDs and reports offline errors` |
| bundle 与质量门禁 | `scripts/acceptance/business/tests/test_release.py` |
| 项目历史、分页、取消及不可变产物 | `scripts/acceptance/project/main.js` |
| 真实 Runner 与业务结果 | `scripts/acceptance/business/main.py --scenario positive` |

## 发布验收

2026-09-09 发布 `6e4f2bf79f21c4f5e2a89b0605323418c1597d76`，两个安装前缀的四个二进制、Server/Node 实际运行文件和内嵌 SPA 均与 bundle 摘要一致。后续 `2a658f43` 仅加强验收工具的结论校验，19 项 Python 3.7 测试通过。

- Rust 全量回归 4,958 passed、0 failed；5 项需真实内核 NFS/runc 的默认忽略测试单独执行全部通过。全量 clippy 与构建通过。
- SPA 499 项测试通过；线上编辑与关联检查 14 项、Agent/DAG/Team/Brain/控制/项目运行全部通过。
- 双节点离线/重启、256 MiB 产物下载、项目 34 次尝试及真实 runc 双向调度通过。真实业务正例完成 API → Runner → Codex → 报告 → 浏览器回放与下载校验；历史缺失 trace 和外部 Go/SDK 失败结论保留。
- NFS 的 103 个资源文件与源目录一致，含 10 个长路径；新任务 44 个资源快照文件完整，663 个历史快照文件未变。56,422 个历史项目产物哈希一致，历史执行记录完整保留；项目规划同步仅刷新节点定义副本的 4 个更新时间。
- Server 执行明细表仍为空，执行索引保持五字段。服务就绪、节点在线且空闲，两个服务重启次数均为 0。无额外观察窗口，无数据库数据删除。

本机完整回执为 `/var/tmp/opencoder-launch-fix-p7o1ih8_/release-receipt.json`，原始日志保存在同目录。稳定契约见 [Agent 平台](../../agent-platform/index.md)、[节点执行](../../../agents/worker/index.md) 和 [Web](../../../agents/web/index.md)。
