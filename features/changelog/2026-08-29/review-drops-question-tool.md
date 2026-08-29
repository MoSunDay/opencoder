Commit: (working-tree, review 去 question 工具 + do-and-done 去 review 描述)

# review 去 question 工具化：question 收敛为 task-plan 专属

## 变更

- **latent 门控收敛**（`crates/session/src/tools/latent.rs`）：`latent_tools_for_skill` 仅 `"task-plan"` 解锁 `question`；`QUESTION_SKILLS` 同步收窄为 `["task-plan"]`。review 不再拥有 question 工具（headless/web 语义不受影响；sandbox 恒可见不变）。
- **review 种子资产**（`crates/core/assets/skills/review/SKILL.md`）：澄清协议改写为「review 不调用 `question` 工具提问；先查再定；查不到 → `assumptions:` 清单显式假设」，并保证正文前 500 字符不含 `task-plan` token（防自名劫持解锁）。
- **do-and-done 种子资产**（`crates/core/assets/skills/do-and-done/SKILL.md`）：去掉对 review 的三处描述（frontmatter description 的 "until review finds..." 从句、停止条件的 "review 以当次证据裁决 go-live ready"、证据要求的 "不得计入 review 的需求完成百分比"），改为自包含证据裁决表述。
- **用户级 skill**（`~/.opencoder/skills/review/SKILL.md`，seed never-clobber 需手工同步）：补「澄清纪律（不用 question 工具）」段，与内置语义对齐。

## 契约测试

- `seeded_task_plan_body_unlocks_question_in_prefix_window`：task-plan 种子必须在前 500 字符自名 + 提及 `question`（解锁前提）。
- `seeded_review_skill_requires_no_question_tool`：review 种子禁含「可在同一轮多问」与 `task-plan` token，必须保留 先查再定/`assumptions:` 守卫。
- `builtin_seed_assets_match_question_gating`（session）：真实 seed 资产 × `unlocked_from_body` 桥接——task-plan 解锁、review 不解锁。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 门控映射 task-plan-only | `skill_to_tool_mapping` | `session/src/tools/latent.rs` |
| review body 不解锁 | `unlocked_from_body_review_skill_unlocks_nothing` | `session/src/tools/latent.rs` |
| review body 不注入 schema | `question_not_unlocked_by_review_skill_body` | `session/tests/latent_tools.rs` |
| LLM 请求边界：review 隐藏 question | `act_with_review_skill_hides_question` | `session/tests/question_gating.rs` |
| 真实种子资产桥接 | `builtin_seed_assets_match_question_gating` | `session/tests/question_gating.rs` |
| token 估算不漂移 | `estimator_*`（review body 零增量断言） | `session/src/tools/mod.rs` |
| task-plan 种子前缀窗口 | `seeded_task_plan_body_unlocks_question_in_prefix_window` | `core/tests/skill_contract.rs` |
| review 种子无 question 契约 | `seeded_review_skill_requires_no_question_tool` | `core/tests/skill_contract.rs` |

- 全量回归：`cargo test --workspace` → 3311 passed / 0 failed（233 个测试二进制，WS-TEST-EXIT=0，当次实跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（CLIPPY-EXIT=0）
