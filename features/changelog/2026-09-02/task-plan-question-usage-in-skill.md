# task-plan skill 内嵌 `question` 完整参数与用法说明

## 背景
`question` 的参数信息此前只存在于工具 JSON schema（`QuestionTool::description()/parameters()`，经 `schema_for()` 下发），task-plan SKILL.md 的「澄清协议」只有行为指引（一句一个、≤4 选项、同轮多问），没有参数名与调用示例——读 skill 文档学不到怎么调。本轮把完整用法说明收进 task-plan skill 文本，使其成为唯一文档面；工具 JSON schema 保持精简（function calling 硬约束不能去）。

## 变更
- **`crates/core/assets/skills/task-plan/SKILL.md`**：「澄清协议」段补全 `question`（string，必填，只放一个问题）与 `options`（string[]，可选，≤4 互斥短候选）参数说明、调用示例（`{"question": ..., "options": [...]}`）与返回值语义（工具结果即用户所选答案或原文答复）；「同一轮可多问」改写为「同一轮可对多个独立决策点分别调用（一次一个最关键问题），全部拿到答复后再收敛计划」。
- **`crates/core/tests/skill_contract.rs::seeded_task_plan_skill_requires_question_tool_guidance`**：新增参数契约断言（`question`（string，必填）/`options`（string[]，可选）/调用示例前缀/返回值语义），沿用 04df804 回归教训把用法说明锁进测试。

## 保持不变
- `QuestionTool::description()` / `parameters()`（`crates/session/src/tools/question.rs`）与 latent 门控（`tools/latent.rs`：plan 免疫、act 需 task-plan 解锁）不动；skill body 前 500 字符窗口契约不受影响（`## 澄清协议（question 工具）` 标题仍在窗口内）。
- review skill 的负向契约（不得含同轮多问承诺）不受影响。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| skill 内嵌用法契约 | `seeded_task_plan_skill_requires_question_tool_guidance` | crates/core/tests/skill_contract.rs |
| seed 落盘含澄清协议 | `seed_builtin_skill_assets_are_fresh`（含「澄清协议」断言） | crates/core/src/skill/seed.rs |
| unlock / 门控行为 | `visibility_*` / `unlocked_from_body` | crates/session/src/tools/latent.rs |
