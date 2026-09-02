# review / task-plan 内置 skill 排版重整（内容零变更）

## 背景

- 用户要求：内置 `review` 与 `task-plan`（含 `references/launch-closure-plan-checklist.md`）**不修改内容，只把组织格式改好看**。
- 原文为密集长段落（一条要素一整段 100+ 字），密度高、层级平；重排为小节 + 短 bullet + 表格，全部子句原样搬运，不增删任何规则语义。

## 变更

- **`crates/core/assets/skills/review/SKILL.md`**：五问逐节改为短 bullet（一子句一行）；「上线结论」判定规则表格化（情形 → 裁决）；证据纪律两条加粗引导词。五问标题、`## 上线结论`、completed/total、向下取整、逻辑本身/变更潜在影响、go-live ready / not ready 等锚点原文保留。
- **`crates/core/assets/skills/task-plan/SKILL.md`**：总则提升为 blockquote；「澄清协议」内 `question` 参数独立 `###` 子节（参数表 + 示例 + 跳过兜底）；五要素每项独立 `### N.` 小节并拆 bullet；P0-P3 优先级分级表；证据成熟度五级箭头链独立成行；核心动作四阶段 bullet 化；「遗漏复查」独立成节。frontmatter 与全部合约锚点（澄清协议 / assumptions: / gating item / 线上 / 生产等价验证 / 做遗漏复查 等）原样保留。
- **`launch-closure-plan-checklist.md`**：1-4 节条目一字未动；`## Plan Output Schema` 两个字段列表合并为一张三列表（字段 × 每项必含 / “列细”时追加）。

## 回归

- 按用户要求本轮不执行测试：纯资产文本变更，无代码逻辑改动；锚点一致性由既有合约测试长期锁（改动前顺手跑过 `cargo test -p opencoder-core --test skill_contract` → 24 passed / 0 failed）。
- latent 解锁契约不受影响：frontmatter `name: task-plan` 未动，legacy 500 字符窗口与 `> Source:` 主路径均安全。
- 内容修正（用户指正）：question 工具不存在「不可用」状态，headless / web 也不会激活 task-plan——删除「`question` 不可用时（headless `run` / web…）」整段表述；跳过兜底语义改挂 TUI 内真实路径「用户跳过答复」（`SKIPPED_REPLY`，`crates/session/src/tools/question.rs`），`assumptions:` 锚点保留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| review 五问 + 逻辑层契约锁 | `seeded_review_skill_requires_five_question_recap` / `seeded_review_skill_requires_no_question_tool` | crates/core/tests/skill_contract.rs |
| task-plan 五要素 + question 用法锁 | `seeded_task_plan_skill_requires_launch_closure_contract` / `seeded_task_plan_skill_requires_question_tool_guidance` | crates/core/tests/skill_contract.rs |
