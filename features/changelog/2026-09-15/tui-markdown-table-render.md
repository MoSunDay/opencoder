# Say 正文 markdown 表格被压成无分隔连续文本修复

症状：Say 正文流式结束后仍「看起来没渲染 markdown」——表格单元格首尾相接挤成一行（如 `CheckOutcomeFinal state...`）、表头单元格里的 `#` 原样露出（形似未渲染的 `# 标题`）。与 2026-09-14 的滞留 open Say 修复（2c774330）不同类：本轮 Say 已 `done:true`、已走 markdown 渲染，问题出在渲染器本身。

根因：`crates/tui/src/markdown.rs` 打开了 `Options::ENABLE_TABLES`，但事件映射对 `Tag::Table/TableHead/TableRow/TableCell` 全部落入 `_ => {}` 兜底——pulldown-cmark 把每个单元格作为独立 inline 序列吐出，渲染器把它们连续 push 进同一段落 span，既无单元格分隔也无换行。glm 输出大量 GFM 表格（验收项/Check&Outcome 等），每张表都被压成不可读的一行。

修复（crates/tui，markdown.rs 414→537 行）：

- `MdRenderer` 增加 `table_active/table_row/table_rows` 缓冲：`Tag::Table` 开表（先 flush 悬空段落），`flush()` 在表内改为把 spans 作为**一个单元格**入行（空单元格也占位，保证列对齐），`TagEnd::TableCell` 落格、`TagEnd::TableRow/TableHead` 收行、`TagEnd::Table` 经 `emit_table` 出行。
- 纯函数 `emit_table`/`table_row_line`/`table_cell_width`：列宽取各列单元格显示宽最大值（`unicode-width`），单元格右补空格对齐，` │ `（muted）分隔；首行（表头）逐 span `patch(BOLD)` 保留 inline 样式；表头后一行 `─×w` 以 `─┼─` 相连的分隔线（总宽与表头严格相等，见下方修正）；残缺行/空表不 panic，缺格补空白格，零格行跳过。
- 不截断不折行（viewport 自行处理超宽）。

实测：将 /root/rdb 真实会话（01M2G6F3…，427 轮、3 万+ text_delta、102 行表格文本）全量事件经 `SessionEvent::from_sse` 重放 ChatView 后 flatten，`Check│Outcome` 表从挤行变为对齐表格。

## 测试覆盖

| 功能 | 测试或证据 |
| --- |
| 基础表：表头 `Check │ Outcome` + BOLD span、`─┼─` 分隔线、正文行按列宽补齐、无 `CheckOutcome` 挤接、无原始 `|---` 行 | `chat::tests::markdown_table::basic_table_header_bold_and_separator`（修复前失败） |
| 单列表/空单元格对齐不 panic | `chat::tests::markdown_table::single_column_and_empty_cells_align_without_panic` |
| 单元格内 inline code span 存活（accent 样式） | `chat::tests::markdown_table::inline_code_span_survives_inside_cells` |
| 分隔线 `┼` 与表头 `│` 按显示宽列位对齐、行宽相等（ASCII + CJK 表） | `chat::tests::markdown_table::separator_cross_aligns_under_column_pipes` |
| Chat e2e：TextDelta 流式表格 + LlmRoundEnd → flatten 含 `│` 表头与 `┼` 分隔线、无 `|---` | `chat::tests::markdown_table::chat_level_say_table_renders_after_llm_round_end` |
| 真实会话重放：/root/rdb 300,940 事件全量重放渲染对齐（本条目实测证据，未入库） | scratch 重放（一次性，已清理） |

## 修正（同日复审 aa8ffd66）：分隔线 `┼` 与列分隔 `│` 列位对齐

复审发现首版分隔线（每列 `─`×(w+2)、单宽 `┼` 相连）总宽恒比表头多 2 列，且每个 `┼` 比对应 `│` 恒偏右 1 列（所有表格可见）。修正 `emit_table`（crates/tui/src/markdown.rs）：每列 `─`×w、以 3 宽 `─┼─` 相连——总宽与表头严格相等，`┼` 落在 `│` 正下方。新增 `separator_cross_aligns_under_column_pipes` 断言列位与行宽（原 4 项测试均未覆盖列位对齐，属盲区）。

已知非阻塞限制（复审记录，暂不修）：
- 表格嵌于 blockquote/列表时容器前缀缩进丢失（`Tag::Table` 先 `flush()` 悬空段落所致；旧代码该场景同样损坏，非回归）。
- `TagEnd::Table` 无条件追加空行，理论空表（`emit_table` 返回空 vec）会遗留孤立空行；GFM 表格必有表头行，实际不可达。

回归：`cargo test -p opencoder-tui` 全绿（lib 1702 项 + 集成测试；say_markdown_e2e / say_pair_dedup / say_raw_repro / say_interleaved_finalize 全过）；`cargo clippy -p opencoder-tui --all-targets` 无新告警。

运维注记：/root/rdb TUI（PID 1713559）仍运行 2026-09-14 22:31 的 pre-fix 平台版本二进制（`a46fff84`），需重启进程方可获得本轮与上轮修复。
