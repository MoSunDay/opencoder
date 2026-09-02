Commit: (working-tree, review 五问回退 + seed 漂移传播)

# review 五问回退 + 内置 skill seeding 漂移传播

## 背景

- `review` skill 在 2026-08-19 被升级为「证据驱动评审」版（7 步 Workflow + Output Shape 模板），按要求回退为 2026-08-18 内置化的**五问版**（五问即产出 + 上线结论裁决）。五问原文无 git 记录（`3876a32` 为无父初始导入，全历史仅 7 步版），故按 8-18 changelog 契约重建而非精确复刻，`say-and-replay` 的五问措辞（目标复述/做了什么/卡点/验证+证据/下一步）作同源参照。
- 评审遗留三项一并收口：① seeded checklist 序号漂移（`51b2dbc` 改资产后 seeded 副本永不自动收敛）；② seed never-clobber 使 skill 修复无法随二进制升级传播；③ `51b2dbc` 轮缺 changelog 条目（补见 [task-plan-checklist-residual-renumber](task-plan-checklist-residual-renumber.md)）。

## 变更

- `crates/core/assets/skills/review/SKILL.md`（98→39 行）：回退为五问版——问一：原始需求目标 / 问二：做了哪些事情及完成度 / 问三：卡点 / 问四：逐项验证+证据 / 问五：下一步 TODO +「上线结论」（五问答完裁决 `go-live ready | not ready`；五问任一缺失或空泛 → 直接 `not ready`）。证据纪律保留：`completed/total` + 百分比（向下取整）、**没有证据 = 没有通过**、证据须**当次实跑**；`assumptions:` 兜底与「不调用 `question` 工具」精简为两行证据纪律。7 步 Workflow（澄清协议节 + Output Shape 模板）整体移除。
- `crates/core/src/skill/seed.rs`：新增 `SeedPolicy` 双策略——内置包 **update-on-drift**（漂移文件先备份 `<file>.user.bak` 再覆盖为 ship 版；同步文件零动作，无 .bak churn），dep-gated 包（ssh-pty/chrome-headless）保持 per-file **never-clobber**。skill 修复自此随二进制升级自动传播到已 seed 机器。
- `crates/core/tests/skill_contract.rs`：`seeded_review_skill_requires_five_question_recap`（五问章节 + 证据纪律 + 无固定模板负向锁）替换 7 步版断言；`seeded_review_skill_requires_no_question_tool` 正向短语随回退改写、两条负向锁（不含「可在同一轮多问」、不含 `task-plan` token——防 hijack question unlock）保留；`seed_builtin_skills_does_not_clobber_existing_files` 改写为 `seed_builtin_skills_backs_up_then_overwrites_user_edits`；`dep_gated_skills_do_not_clobber_existing` 扩展无 `.bak` 断言。
- `agents/core/index.md` / `features/index.md`：review 职责描述与 seeding 语义行同步。

## 回归（当次实跑）

- `cargo test -p opencoder-core`：**322 passed / 0 failed**。
- `cargo test -p opencoder-session`：**756 passed / 0 failed**。
- `cargo clippy -p opencoder-core --all-targets -- -D warnings`：零警告。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `seeded_review_skill_requires_five_question_recap` | integration | 问一~问五五章节 + 上线结论 + completed/total/向下取整/没有证据=没有通过/当次实跑/go-live 裁决 + 无 Output Shape/goal: 模板残留 |
| `seeded_review_skill_requires_no_question_tool` | integration | 不把提问当侦察手段 + assumptions: + 不调用 question 工具；负向锁（无「可在同一轮多问」、无 task-plan token）保留 |
| `seed_builtin_skills_backs_up_then_overwrites_user_edits` | integration | 漂移即覆盖为 ship 版 + 用户编辑落 `<file>.user.bak` + 已移除引用文件不复活 |
| `builtin_seed_overwrites_drift_and_backs_up_user_edit` | unit | 同上语义（seed.rs 内单测，断言 .bak 内容为用户原文） |
| `builtin_seed_is_idempotent_without_backup_churn` | unit | 同步文件重 seed 不产生 `.bak` |
| `dep_gated_seed_never_clobbers_and_never_backs_up` | unit | dep-gated 永不覆盖、无 `.bak` |
| `dep_gated_skills_do_not_clobber_existing` | integration | dep-gated 用户改动存活 + 无 `.bak` |
