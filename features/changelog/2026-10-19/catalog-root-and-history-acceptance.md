Commit: 3d17733ea3b4660abf1736681d9c0b05e2e422c2

# 默认资源目录保护与历史读取验收

DAG 和二进制删除检查现在使用与 Agent 资源 API 相同的根目录解析。未显式配置目录时，环境变量与默认目录中的引用仍会阻止删除；无法确认目录或文件时拒绝清理。

历史读取验收必须固定原执行与一个退役 Runtime，并在读取前后验证归属、配置和实际进程身份。缺失归属、当前 Runtime、错误执行或 schema、HTTP 失败以及没有真实进程的样本均不能通过；读取失败不换样本、不静默重试。

| 行为 | 定向测试 |
| --- | --- |
| 默认目录保护 DAG、二进制，移除引用后允许删除 | `default_agent_root_protects_dag_and_binary_until_reference_is_removed` |
| 默认目录缺失、错误类型和无法读取文件时拒绝删除 | `unavailable_default_agent_root_and_unreadable_files_block_deletion` |
| 固定退役归属并记录真实进程，读取不改数据库 | `test_pinned_retired_owner_records_real_process_and_unchanged_database` |
| 缺失、错误或当前 Runtime 不能代替旧归属 | `test_missing_owner_is_rejected_before_reading`、`test_different_owner_is_rejected_before_reading`、`test_active_owner_is_rejected_before_reading`、`test_current_runtime_cannot_stand_in_for_historical_runtime`、`test_unknown_current_runtime_cannot_establish_old_ownership` |
| 归属变化、进程缺失、错误响应或 HTTP 失败不能通过 | `test_owner_changes_during_read_are_rejected`、`test_missing_process_identity_cannot_pass`、`test_response_must_match_execution_and_schema`、`test_http_failure_is_not_retried_or_replaced` |
| 验收数据库不存在时拒绝且不创建文件 | `test_missing_database_does_not_create_one` |

HTTP 测试夹具建立其配置的资源目录；历史名称用例先确认活动执行阻止删除，再由节点报告完成后验证删除仍保留历史名称。

| 相关 HTTP 行为 | 定向测试 |
| --- | --- |
| DAG 定义增删改查 | `dag_definitions_crud` |
| 活动执行保护与终态历史名称保留 | `execution_list_lifts_dispatch_time_names` |

定向验证：目录保护集成测试 24 项、历史归属验收测试 11 项及相关 DAG HTTP 测试 2 项通过；`cargo clippy -p opencoder-control --all-targets -- -D warnings` 零警告。未运行全量测试；发布包验证与稳定观察单独保存回执。

相关说明：[控制面](../../../agents/control/index.md)、[Host 与 Runtime](../../../agents/agent/index.md)。
