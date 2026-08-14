# plan 模式结构化提问：`question` 工具 + TUI 对话框 + 同轮回填

## 背景
plan 模式此前只能靠模型在正文里问问题、等用户手动回复（多耗一整轮 user 消息 + plan tag 后缀）。本轮给 plan agent 增加结构化提问能力：模型调用新 `question` 工具 → TUI 弹出对话框（预设选项 + 自定义输入口）→ 用户作答作为 tool result **同轮回填**，驱动模型继续生成计划。token 增量受控：schema 仅 2 参数、只注入 plan agent（act/explore/build/command 零增量）。

## 变更

### session 层
- **`crates/session/src/tools/question.rs`（新增，334 行）**：`QuestionHub`（`attach/ask/resolve/abandon`，`Arc<Mutex<HubState>>`：waiting oneshot 表 + early 答案表——`ToolStart` 先于工具执行 emit，UI 可能先 resolve，early 表保证两种时序都成立）+ `QuestionTool`。未 attach（headless run / web）立即返回固定文案 `NO_LISTENER_REPLY`，绝不挂起；attached 时 `ask` 注册 oneshot 并 await，`AskGuard`（Drop）在 future 被 cancel 竞速丢弃时清理注册，无泄漏。
- **`tools/mod.rs`**：`question` 注册进 `registry()`（占位 hub，仅 schema/token 估算用）；**`runner/mod.rs::build_full_registry`** 用 `session.question_hub` 覆盖重绑——答案直连共享 hub，不走 `UiCmd`（`process_cmd(Prompt)` 整 turn await，排队答案会死锁）。
- **`SessionState`**：`pub question_hub: Arc<QuestionHub>`（`new` 内建 + `with_question_hub` builder + `resume.rs` 构造点补齐）。
- **`runner/execute.rs::leaf_tool_timeout`**：`question` 同 bash 豁免 600s 保险丝（等人不限时，cancel 是唯一出口）。

### core 层（token 精炼）
- **`agent.rs`**：plan agent `ToolFilter::Allow += question`（其余 agent 不动 → 零 schema 增量）；`PLAN_SUFFIX` 措辞指向 `question` 工具（"at most one per turn"约束），长度基本不变。
- 复用 `ToolStart/ToolEnd` 事件（问题=ToolStart.input，答案=ToolEnd.output），不加新 `SessionEvent` 变体——持久化/replay/SSE 免费兼容。

### TUI 层
- **`question_menu/`（新增：mod/state/view，共 ~690 行）**：`QuestionMenu` 纯键位状态机（↑↓ 选、Enter 提交、末行 "✎ custom…" / Tab 聚焦自由输入框——**自定义文本优先**、Esc=跳过回填 `SKIP_ANSWER`、Ctrl+D 同 Esc）；composer 顶边锚定弹窗 + 自定义框聚焦时 `set_cursor_position`（光标门控进 `render.rs`：question_menu 打开时抑制 composer 光标）。并行多问排队（`VecDeque<QuestionPrompt>`，逐个弹）。
- **`app_loop.rs::fold_ui_events`**：live `ToolStart{name=="question"}` 入队开弹窗、对应 `ToolEnd{id}` 关闭（replay 不触发弹窗）。**`app.rs`**：hub attach（worker spawn 前）+ 弹窗键位链插入（mcp 之后）+ `/task` 切换时弃答（abandon → 工具端收 skip 文案，不悬空）；`switch_session` 重绑新会话 hub。`chat_helpers::summarize` 键列表加 `question`（chat 区 `▸ question` 头显示问句）。
- 为守 800 行上限，app.rs 的 `KeyAction::SetSkill` arm 提取为 `skill_persist::apply_skill_selection`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| hub ask→resolve 送达 | `ask_then_resolve_delivers_the_answer` | session/src/tools/question.rs |
| resolve 先于 ask（early 表） | `resolve_before_ask_is_parked_and_consumed_once` | session/src/tools/question.rs |
| abandon 关通道无 early 残留 | `abandon_closes_the_channel_without_early_residue` | session/src/tools/question.rs |
| attach 开关 | `attach_flag_toggles` | session/src/tools/question.rs |
| 未 attach 立即兜底 | `execute_without_listener_returns_fallback_immediately` | session/src/tools/question.rs |
| 缺 question 参数报错 | `execute_missing_question_is_an_error` | session/src/tools/question.rs |
| 答案成为 tool result | `execute_resolved_answer_becomes_the_tool_result` | session/src/tools/question.rs |
| cancel 竞速无泄漏 | `cancelled_future_does_not_leak_registration` | session/src/tools/question.rs |
| 集成：答案进二轮上下文 | `answered_question_feeds_the_followup_call` | session/tests/question_tool.rs |
| 集成：skip 文案回填 | `skipped_question_returns_the_skip_text` | session/tests/question_tool.rs |
| 集成：headless 兜底不挂起 | `unattached_hub_falls_back_without_waiting` | session/tests/question_tool.rs |
| 集成：turn cancel 不悬空 | `turn_cancel_unblocks_a_pending_question` | session/tests/question_tool.rs |
| 保险丝豁免 | `leaf_tool_timeout_exempts_question` | session/src/runner/execute_timeout_tests.rs |
| plan-only + schema 紧凑 | `question_tool_is_plan_agent_only` / `question_schema_is_plan_only_and_compact` | core/src/agent.rs / session/src/tools/mod.rs |
| 键位状态机 ×9 | `down_then_enter_answers_the_selected_option` 等 | tui/src/question_menu/state.rs |
| 弹窗渲染（TestBackend） | `popup_shows_question_options_and_hint` / `popup_respects_composer_top_anchor` | tui/src/question_menu/view.rs |
| 事件→弹窗胶水 ×5 | `tool_start_opens_then_queues_parallel_questions` 等 | tui/src/question_menu/mod.rs |
| worker 级全流程 | `worker_prompt_with_question_resolved_mid_turn` | tui/tests/question_flow.rs |

（隔离重验：`git worktree /tmp/oc-question @00b888f` + 仅本任务 32 文件白名单，workspace **2481 passed / 0 failed = 基线 2449 + 本轮新增 32**（153 suites，归属精确）；clippy `-D warnings` 零警告、`cargo build` 零错误同树实跑。混合树快照 2527 为含并发会话 ~46 项的污染计数，不作为本任务依据。question schema 实测 133 token、<200 上限由 `question_schema_is_plan_only_and_compact` 钉住。）

## 风险与后续
- 一轮并行多问：队列逐弹 + PLAN_SUFFIX "至多一个" 约束。
- web UI 对话框不在本轮范围（SSE 可见 tool_start/end；hub 未 attach → 兜底文案）。
- act 模式启用 = `agent.rs` Allow 一行（保持现状：仅 plan）。
