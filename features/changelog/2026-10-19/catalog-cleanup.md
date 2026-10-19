Commit: f6768ad434907a739f0271f52cdc3f9c991a40aa

# 能力与 DAG 目录去重

目录重复入口会干扰选择，重复上传也会增加无用资源。Server 统一检查写入冲突和删除引用，维护命令按明确清单备份、条件删除和逐项恢复。

- 能力选择目录保留注册说明与等价来源，执行仍能解析原 ID；不同输入输出约定不合并。
- DAG 同内容重试不改变时间；异名同定义、二进制相同内容返回已有条目，前端保留草稿并支持查看详情。
- 删除与引用写入共用跨进程锁，引用不完整或内容变化时拒绝；历史执行和鉴权数据不变。
- 清理默认保留最近 7 天创建、更新或受理执行的独立条目，仅处理定义与二进制池，不配置自动调度。备份校验全部版本及字节摘要，恢复只处理指定对象，不回滚数据库。
- 发布探针复用已有相同字节的固定版本，避免每次演练生成相同资源。
- 任务受理和引用写入可等待目录修改最多 30 秒，超时返回可重试的 503；清理请求之间留出读取机会，避免连续删除让任务受理超时。

## 验证映射

| 行为 | 定向验证 |
| --- | --- |
| 幂等、异名冲突、跨 Server 并发 | `catalog_guard::definitions`、`catalog_guard::binaries` |
| 保存版本、活动执行、日程、文件与晚到引用保护 | `catalog_guard::references` |
| 慢目录修改期间的受理与引用写入 | `admission_and_reference_writes_wait_through_a_slow_catalog_mutation` |
| 目录折叠及实际 Agent 资源约束 | `catalog_guard::library`、Control `catalog` 单元测试 |
| 独立资源代理及原始下载 | `resource_service_native`、`dag_binary_scope` |
| 能力绑定、DAG 定义及定时提交兼容 | e2e `brain_api`、`teams_dag_defs::dag`、`schedule_api` |
| 旧计划沿原能力 ID 保存、新建选项去重 | `selection.test.js`、`milestone/editor.dom.test.jsx` |
| 保留草稿及查看冲突条目 | `conflict.dom.test.jsx` |
| 清理边界、备份恢复、条件拒绝及断点续跑 | `test_cleanup.py`、`test_apply.py` |
| 发布探针复用 | `rolling_tests/test_native.py` |

构建、发布与真实原生任务连续性以独立发布回执为准；定向测试不代表全量回归。

相关：[DAG](../../dag/index.md)、[能力库](../../brain/index.md)、[Control](../../../agents/control/index.md)、[维护命令](../../../scripts/maintenance/catalog_cleanup/README.md)。
