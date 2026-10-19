Commit: 060b4a24

# 能力选择去重与内容更新保护

完整能力目录继续支持已有 ID；新增 `selection_ids` 统一 Web 与 TUI 的新选择。仅隐藏与登记项执行定义、类型、目标、必填输入输出一致的自动项，保留所有登记项、内置项及固定计划版本。

能力详情新增 `content_version`，更新请求可选 `expected_content_version`；事务内发现版本变化返回 409，文字、样例和向量均不覆盖。旧请求兼容，embedding 失败仍不写入。

目录去重读取 Agent 资源文件并计算摘要时，使用独立阻塞线程并显式保留资源根；耗时文件读取不再占用处理健康检查及执行准入的异步线程。

| 功能 | 定向测试 |
| --- | --- |
| 来源与输入输出差异 | `catalog::selection::tests` |
| 完整目录、TUI 与旧 ID 保存 | `library_and_tui_share_selection_while_hidden_ids_remain_valid` |
| HTTP 409 与旧调用兼容 | `guarded_update_returns_conflict_without_overwriting_and_legacy_still_works` |
| 同时间戳更新、删除与向量保护 | `stale_content_cannot_replace_examples_or_vector_even_with_same_timestamp` |
| embedding 期间并发编辑 | `concurrent_edit_during_embedding_is_rejected_at_commit` |
| 选择标签及旧 ID 保存 | `selection.test.js`、`milestone/editor.dom.test.jsx` |
| 原分页与完整列展示 | `brainPanel.pagination.dom.test.jsx`、`brainPanel.dom.test.jsx` |
| 慢资源读取期间其他请求继续运行，资源根不丢失 | `slow_resource_reads_yield_to_requests_and_keep_their_root` |
| 资源内容、执行配置差异和旧目录 ID 保留 | `catalog_guard::library` |

接口与行为见 [大脑工作台](../../brain/index.md)。线上模型与地址只由运行配置提供，仓库不保存实际接入凭据或数据。
