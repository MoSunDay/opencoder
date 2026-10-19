Commit: c139569c708859863cadc589ce49a6dcaf371f13

# Agent 删除引用保护

Agent 卡片删除现在与目录引用写入互斥，并持有资源写锁直到完成。已保存计划、能力绑定、日程、TODO、未完成执行、待派发回执、配置与资源文件中的引用会阻止删除。资源池元数据的自身名称、Agent 卡片的共享池链接不作为其他 Agent 引用；未知元数据字段仍检查，无法确认的文件继续阻止清理。

详情返回 ETag，条件删除在锁内复核；旧客户端省略 If-Match 时继续可用。删除标记阻止晚到请求引用已删除卡片；原名重建后恢复使用。共享资源池、已完成执行历史和鉴权数据不变。健康接口增加 `agent_delete_guard: 1`，保留原有 `catalog_guard: 2`。

| 行为 | 定向测试 |
| --- | --- |
| 内容版本冲突、旧客户端调用与内置卡片保护 | `agent_conditional_delete_rechecks_card_and_preserves_legacy_calls` |
| 历史别名、活动执行引用与已完成记录保留 | `agent_saved_alias_and_active_execution_block_delete_without_touching_history` |
| 并发引用写入后重新核对 | `agent_delete_waits_for_reference_writer_and_rechecks` |
| 删除后拒绝晚到日程、DAG 和执行；原名重建恢复 | `agent_removal_rejects_late_schedule_dag_and_admission_until_recreated` |
| 资源池自身名称、文件引用与无法读取的文件 | `agent_resource_scan_distinguishes_pool_identity_and_protects_unreadable_files` |
| 资源写锁与目录检查协作 | `agent_file_writer_lock_is_held_through_reference_validation` |
| 共用资源池不误判为 Agent 引用，未知依赖继续阻止删除 | `agent_card_pool_links_do_not_block_card_removal_but_unknown_dependencies_do` |
| Agent/Operator 绑定、旧能力 ID 与上传文件引用 | `removal_markers_match_agent_bindings_without_rejecting_unrelated_text` |

定向验证：目录保护初次 21 项通过；修正共享资源判断后 7 项 Agent 用例通过（含 6 项重跑、1 项新增），另有引用识别单测 5 项、既有 Agent API 生命周期 1 项通过，共 28 个不同 Rust 用例；`cargo clippy -p opencoder-control -p opencoder-agents --all-targets -- -D warnings` 零警告。未运行全量测试；发布与稳定观察单独验收。

相关说明：[Control](../../../agents/control/index.md)、[Agent 资源](../../../agents/agents/index.md)、[能力库](../../brain/index.md)。
