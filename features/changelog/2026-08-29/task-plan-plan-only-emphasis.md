Commit: (working-tree, task-plan 强调只规划不执行——交付 0 到上线完整全局计划)

# task-plan 强调：聚焦从 0 到交付的完整全局 plan，先不执行

## 背景

上一轮把 question 收敛进 task-plan、去掉了 Any Home 死协议；本轮按用户要求进一步锚定该 skill 的交付语义：task-plan 的产出物是唯一一份覆盖从 0（当前现状）到交付/可上线全路径的全局计划，本轮调用只规划、不执行——避免 skill 被调用后立刻滑进实现。

## 实现（仅 `crates/core/assets/skills/task-plan/SKILL.md`，本地 seed 副本同步、保持逐字节一致）

- frontmatter description 追加「输出只规划、不执行的完整闭环计划」，激活期即校准预期。
- Overview 收尾句明确交付物：唯一一份从 0 到交付/可上线全路径的全局计划；只规划，不执行。
- 重写「当用户调用本 Skill 时」段：交付物就是计划本身；计划交付后即停，等用户确认或拿到明确执行指令再落地；用户要求「边规划边修复」时也必须先完整产出计划（含关键路径与 P0 顺序），经确认后再推进，不允许跳过计划直接动手。
- When-To-Use 尾句同步：要求直接修复时仍先交付完整计划并标注关键路径，经确认后进入修复。
- 约束保持：`question` 字样仍处于注入 body 前 500 字符解锁窗内（编辑后实测 index 360/路径 60 字符时 375，< 500），latent 解锁不受影响。

## 测试

- `cargo test -p opencoder-core --test skill_contract` → 23 passed / 0 failed（含 `seeded_task_plan_body_unlocks_question_in_prefix_window`、Any Home 不回种、review 无 question 守护）
- `cargo test -p opencoder-session --test question_gating` → 6 passed / 0 failed（真实种子资产端到端：无 skill 隐藏、task-plan 解锁 act/sandbox、review 不解锁）
- 本轮无 Rust 代码变更；workspace 级 clippy 门禁前置（shellguard 收口）已解除并终验复绿：`cargo clippy --workspace --all-targets -- -D warnings` → 0 告警（见同日 task-plan-drops-anyhome-question-latent-only.md 终验补录）。
