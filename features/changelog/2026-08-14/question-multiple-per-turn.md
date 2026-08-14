Commit: (working-tree, pre-initial-commit)

# plan 模式 question 工具放开「每轮至多一问」：一轮可批量澄清 + 存疑点对齐

## 背景
`question` 工具的提示词层仍有「at most one question per turn」约束，但运行时早已支持一轮多问：`QuestionHub` 的 waiting 表按 message_id 并发注册、TUI 逐弹队列渲染、headless/web 每次调用独立回兜底文案。本轮仅放开文案约束并钉住新语义（含一点补充：明确「存在疑惑的点」也应通过 question 对齐，而非默默假设）。

## 变更
- **`crates/session/src/tools/question.rs`**：`QuestionTool::description` 去掉 "one clarifying question / at most one question per turn"，改为 "you may ask several in one turn (one per call)"；"Use ONLY when genuinely ambiguous" 门控措辞保留（防过度提问），schema 结构不动（多问 = 一轮多次调用）。
- **`crates/core/src/agent.rs`**：
  - `PLAN_SUFFIX`：`(at most one per turn)` → `or you have doubts, align via the \`question\` tool (you may ask several in one turn)` —— 同时落实「存疑点必须通过 question 对齐，不要默默假设」；措辞近等长，question schema `<200 token` 预算断言不变。
  - plan agent Allow 列表上方注释同步新语义（several per turn, one per call）。
- **不改**：agent.rs 的 act Allow、`QuestionHub`/TUI 队列/headless 兜底行为面、schema 参数结构。

## 测试清单（rules/01/02）
| 语义 | 测试 | 位置 |
| --- | --- | --- |
| PLAN_SUFFIX 无 "at most one" + 含批量措辞 + 存疑对齐 | `plan_prompt_allows_multiple_questions_per_turn`（新增） | core/src/agent.rs |
| description 无每轮上限 + 含批量措辞 + 门控保留 | `description_allows_several_questions_per_turn`（新增） | session/src/tools/question.rs |
| plan-only + schema 紧凑（<200 token）不回归 | `question_tool_is_plan_agent_only` / `question_schema_is_plan_only_and_compact` | core/src/agent.rs / session/src/tools/mod.rs |
| TUI 并发队列行为面不变 | `tool_start_opens_then_queues_parallel_questions` | tui/src/question_menu/mod.rs |
| hub 并发/兜底/cancel 语义不回归 | `ask_then_resolve_delivers_the_answer` 等 ×7 | session/src/tools/question.rs、session/tests/question_tool.rs |

- 全量回归：`cargo test --workspace` → 全绿（本轮用户确认刚跑过、免重复执行）；clippy `-D warnings` 零警告。
- 行数：agent.rs 298 ≤ 800（迭代中）/ question.rs 345 ≤ 800（迭代中）。

## Impact Surface
- 用户可感知：plan 模式一轮内可收到多个澄清问题（TUI 逐弹队列、headless/web 各自兜底），存疑点不再被默默假设。
- 不影响：act/explore/build agent（仍无 question 工具）、schema 结构、`QuestionHub` 并发/兜底/cancel 行为、CLI/Web/store 边界。

## 风险与后续
- 过度提问：靠 "genuinely ambiguous" 门控措辞约束，无硬上限（已确认的设计取舍）。
- headless/web：多次调用各回一次兜底文案，无害。
- 历史 changelog（plan-question-tool.md）中 "at most one per turn" 为当时记录，不改写。

## Related Docs
- [agents/session](../../agents/session/index.md)、[agents/core](../../agents/core/index.md)
- [2026-08-14 plan-question-tool（前序，引入每轮一问约束）](plan-question-tool.md)
