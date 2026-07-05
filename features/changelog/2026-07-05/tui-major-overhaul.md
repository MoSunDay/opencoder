# TUI 大改：Thinking 折叠块 + Markdown 渲染 + 多行输入框 + 滚动条 + /task 多会话

## 变更摘要

### 正文内容层
- **ChatBlock 模型**：`ChatView` 从 `Vec<Line>` 重构为 `Vec<ChatBlock>`。四种块类型：
  - `Marker` — 用户输入、系统标记（加粗/彩色单行）
  - `Assistant { raw, rendered, done }` — 流式时显示纯文本（低延迟），`Done` 时一次性 Markdown 渲染
  - `Thinking { text, collapsed }` — 推理内容，暗色斜体，可折叠（点击表头切换）
  - `Tool { header, output }` — 工具调用，截断输出
- **Thinking 折叠块**：`ReasoningDelta` 不再丢弃。推理内容以 `\u{1f4ad} Thinking` 表头 + 暗色斜体样式展示，`toggle_last_thinking()` 切换折叠/展开。折叠时显示 `(N lines)` 摘要。
- **Markdown 渲染**：新增 `markdown.rs` 模块，基于 `pulldown-cmark`。支持：H1-H3 标题（彩色加粗）、代码块（带 `┌──┐` 围栏 + 语言标签）、粗体/斜体/删除线、有序/无序列表、行内代码、引用块、链接、水平线。渲染仅在 `Done` 时触发一次——流式期间纯文本直出，不打破高性能目标。

### 布局与输入框
- **动态布局**：5 行布局替代原 3 行——body（`Min(3)`）+ queue 面板（条件）+ skill 下拉（条件）+ composer（动态高度）+ status（1 行）。Composer 最少 2 行内容高度，随输入自动增高（上限 1/3 屏幕高度）。
- **多行输入框**：Shift+Enter / Alt+Enter 插入换行（Kitty keyboard enhancement best-effort）。Up/Down 在多行模式下垂直移动光标，单行时保持历史导航。`composer.rs` 新增 `cursor_row_col`、`move_cursor_vertical`、`insert_newline`、`display_rows` 四个纯函数 + 单元测试。
- **滚动条修复**：右侧预留 1 列 gutter，滚动条不再覆盖文本。短内容（`total_rows <= visible_h`）时完全隐藏滚动条。`ScrollbarState` 不再用 `.max(visible_h)` hack。

### Skill 下拉框
- 从居中弹窗改为锚定在 composer 上方的下拉框（`render_skill_in_rect`），与 queue 面板一起占据 composer 上方的动态行。

### Queue/Steer 面板
- steer/queue 计数从 status bar 上移至 composer 上方的独立面板（蓝色 steer、黄色 queue）。
- Store API 新增 `delete_input(seq)` — 删除待处理 input 行。

### /task 多会话切换
- 新增 `task.rs` 模块 — `TaskPicker`（会话选择器）+ `handle_task_key` + `render_task_picker`。
- `/` 在空输入框触发 task picker 弹窗，列出 store 中的历史会话 + "New task" 选项。
- 选择会话：旧 worker 退出，新 SessionState 创建（新会话）或从 store 加载消息（恢复会话），ChatView 重置/重建。
- 会话切换在同一终端会话内完成，无需重启 TUI。

### 模块拆分
- `app.rs` 从 777 行降至 692 行。渲染函数提取至 `render.rs`（306 行）。新增 `markdown.rs`（215 行）、`task.rs`（155 行）。

## Follow-up（上线前 review 修复）
- **包裹行计数**：使用 ratatui 自带 `Paragraph::line_count`（同 WordWrapper），删除自写 `wrapped_metrics`。
- **Markdown 渲染时机**：流式期间纯文本，`Done` 后一次性 Markdown 渲染——不打破高性能。
- **Kitty keyboard enhancement**：best-effort 启用，不支持的终端静默降级（Enter=提交）。

## 测试
- 全量 171 passed，`clippy --all-targets -- -D warnings` 全绿。
- Release 二进制 9.9 MB（stripped），已安装至 `/usr/local/bin/opencoder` + `/root/.cargo/bin/opencoder`。

## Related Docs
- [skill-picker changelog](./skill-picker.md) — skill 下拉框复用其 SkillMenu 逻辑。
- [tui-context-mixing-scroll-quit](./tui-context-mixing-scroll-quit.md) — 滚动条 + 布局基础在此基础上演进。
