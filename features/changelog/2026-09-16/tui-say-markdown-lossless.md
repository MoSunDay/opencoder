Commit: 92b4ec156acd78b62031f257c3099863b6cda6b3

# Say 头部保留 Markdown 样式，正文传递不再丢片段

`Say(n step): xxx` 完成后格式异常有两条独立路径，不能只检查 `Assistant.done`：

- `say_preview_for` 将渲染后的首行拼回 `String`，头部再用 `Span::raw` 输出。粗体、斜体、删除线和代码颜色因此丢失；正文去重又隐藏首行，单行回复没有其他地方保留样式。
- worker 在 UI 通道剩余容量不超过 64 时丢弃父级 `TextDelta`，但 `AssistantFinal` 只覆盖最后一条回复。中间轮次或中断输出丢失 Markdown 定界符、换行后，即使 `done=true`，损坏的原文也无法正确解析。

修复：

- 合并头部使用首个可见渲染行的完整样式片段，只裁掉外侧空白；没有可见内容的 Markdown 不回退显示原始源码。
- 复制模式拼接全部预览片段，避免保留样式后只复制第一段文字。
- 将 worker 事件桥接提取到 `worker/delivery.rs`，改为有序、无损传递。在 UI 背压下合并相邻正文片段，按事件数和文本大小限制批处理；思考、重试、子代理、重置和结束事件均保留原有顺序。
- 用正文完整性测试替换原先要求丢弃正文的测试，并保留原有生命周期可靠交付测试。

验证：修复前新增样式与压力回归共 4 项失败；修复后 1,705 项 TUI 单元测试及 18 项相关集成测试通过，`cargo clippy -p opencoder-tui --lib --tests -- -D warnings` 与变更文件格式检查通过。新增覆盖逐字符分块、CRLF、代码块、表格、不同通道容量、完成/错误/中断、重试交错、子代理/重置边界、终端换行后的样式和复制内容。真实 worker 的多轮测试同时检查中间回复与最终回复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 头部样式及完整复制 | `completed_say_header_preserves_markdown_styles_and_copy_payload` | `crates/tui/src/chat_tests/say_markdown_e2e.rs` |
| 不可见 Markdown 与终端换行样式 | `completed_say_with_no_visible_markdown_does_not_reintroduce_source`、`completed_say_styles_survive_terminal_wrapping` | 同上 |
| worker 多轮输出完整性 | `final_say_is_markdown_rendered_after_turn_done` | 同上 |
| 拥堵下中间回复的定界符完整性 | `pressure_cannot_strip_markdown_delimiters_from_an_interim_say` | `crates/tui/src/worker/delivery_tests.rs` |
| 逐字符分块、通道容量与终态 | `say_markdown_survives_pressure_at_every_character_and_terminal_boundary` | 同上 |
| 重试与思考交错顺序 | `say_retry_and_interleaved_reasoning_are_ordering_barriers` | 同上 |
| 批处理及子代理/重置边界 | `pressure_coalesces_backlog_without_crossing_child_or_reset_events` | 同上 |

工作区门禁：`cargo clippy --workspace --all-targets -- -D warnings` 通过。`cargo test --workspace -j 4` 未完成：停在 `opencoder-agent` 的 `host::tests::three_runtime_versions_keep_live_model_calls_and_global_fifo`，超过两分钟仍无结果；同一测试二进制单跑 45 秒也超时（exit 124），随后终止本次挂起的测试进程。该包不依赖 TUI，不将此次全量回归记为通过。日志：`/tmp/opencoder-say-workspace-tests.log`、`/tmp/opencoder-say-host-probe.log`。

`cargo build --workspace -j 4` 未完成：共享缓存与仓库 `target` 均被其他回归任务持有构建锁；独立缓存复制受到磁盘争用，已停止并清理本次临时副本。TUI 单元及集成测试目标已成功编译，但不将其代替工作区构建门禁。日志：`/tmp/opencoder-say-workspace-build.log`。

相关逻辑：[TUI 模块](../../../agents/tui/index.md)。
