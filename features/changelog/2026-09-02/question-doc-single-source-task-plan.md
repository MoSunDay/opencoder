# question 工具描述单一文档面收敛——base prompt 与工具 schema 去描述

## 背景
上一轮（[task-plan-question-usage-in-skill](2026-09-02/task-plan-question-usage-in-skill.md)）已把 `question` 完整参数与用法收进 task-plan SKILL.md「澄清协议」段，但两处旧描述仍在重复下发：plan 基础提示词 `PLAN_SUFFIX` 尾句（prefer asking over assuming / several in one turn / looked up first）与 `QuestionTool::description()` 的协议性长描述。同一协议三处维护必然漂移。本轮收敛：**task-plan skill 文本是 `question` 的唯一描述面**，其余全部移除。

## 变更
- **`crates/core/src/agent.rs`**：`PLAN_SUFFIX` 删除 question 指引句（只保留只读拦截与 explore 委派）；plan agent 工具表注释与测试注释同步去指引化。测试 `plan_prompt_is_read_only_with_question_guidance` 翻转为 `plan_prompt_is_read_only_without_question_advertisement`——断言 plan prompt 不含 `` `question` `` 反引克工具名及三句指引文案。
- **`crates/session/src/tools/question.rs`**：`description()` 收窄为一句标识 + 指针（"Ask the user a clarifying question. Usage guidance: task-plan skill."）；模块 doc 注明唯一文档面。测试 `description_allows_several_questions_per_turn` 重写为 `description_defers_guidance_to_task_plan_skill`——锁死 schema 不回涨协议文案（prefer asking / several in one turn / look up 等均禁）。
- **stale 注释清理**（表述从「clarification protocol lives in the base prompt」改为「plan-kind parity exemption，指引在 task-plan skill 文本」）：`tools/latent.rs`（模块头 + `is_visible` doc）、`tools/mod.rs` 测试注释、`runner/llm_call.rs` 过滤注释、`tests/latent_tools.rs` / `tests/question_gating.rs` 文件头。

## 保持不变
- task-plan SKILL.md「澄清协议（question 工具）」段与参数表/调用示例/跳过兜底——唯一文档面，`skill_contract.rs::seeded_task_plan_skill_requires_question_tool_guidance` 继续锁定。
- latent 门控行为（plan 免疫、act 需 task-plan 正文前 500 字符解锁、review 不解锁）与 `parameters()` JSON schema（function calling 硬约束）不动。
- do-and-done「暂停协议」只描述自身暂停行为（不指名 question 工具），review SKILL.md 已无 question 引用，均不动。

## Validation（当次实跑）
- `cargo test --workspace --no-fail-fast`：全绿（套件数与通过数见下）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo fmt --all --check`：干净。

## 测试覆盖表
| 测试 | 层 | 覆盖点 |
|---|---|---|
| `agent::tests::plan_prompt_is_read_only_without_question_advertisement` | unit | plan 基础提示词零 question 广告（反引克名 + 三句指引全禁），只读约束保留 |
| `tools::question::tests::description_defers_guidance_to_task_plan_skill` | unit | schema 描述=标识+指针，协议文案禁入；文档面唯一性 |
| `skill_contract.rs::seeded_task_plan_skill_requires_question_tool_guidance` | integration | task-plan 文本持续携带完整参数/示例/兜底（未改动，回归确认） |
| `tools::tests::question_schema_is_plan_only_and_compact` / `latent::*` / `question_gating` / `latent_tools` | unit/integration | 门控矩阵与 schema 成本回归（未改动，确认不受描述收窄影响） |

纯提示词/描述收敛 + 注释与测试对齐，无删测试 / 无 `#[ignore]` / 无弱断言 / 无密钥。
