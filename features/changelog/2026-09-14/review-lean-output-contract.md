# review skill 精简输出契约（严格五问 + 板块之外零输出）

## 背景

- 用户反馈：review 实际输出含大量无意义冗余内容，要求**严格按照五问执行**，只输出 TODO List、进展、验证手段与证据等关键信息。
- 根因：`review` SKILL.md 原文写着「答好五问本身就是产出，没有固定输出模板」——无结构约束导致模型自由发挥：开篇引言、过程叙述、整段搬运 diff、跨板块复读目标/清单等冗余填充。

## 变更

- **`crates/core/assets/skills/review/SKILL.md`**（53 → 61 行）：
  - 删除「没有固定输出模板」自由发挥口子，新增 `## 输出契约`：输出 = 问一 ~ 问五 + 上线结论**六个板块、顺序固定、板块之外零输出**。
  - 冗余禁令四条：不写开篇引言/过程叙述/方法论自述/结尾展望/客套；一行一条关键信息（结论在前、证据在后）；不整段粘贴 diff/代码/文件内容（只给 `file:line` + 一行说明）；不跨板块复读（问四只按问二序号逐项核查）。
  - 问二改 TODO 清单式盘点：一行一项（做了什么 + 交付物位置 `file:line`），完成度 `completed/total` + 百分比（向下取整）一行给出；未完成项只标状态。
  - 问四固化每项两行格式：**逻辑本身**（验证方式 = 当次 diff 对照 / 调用链追溯，证据 = `file:line`）+ **变更潜在影响**（一行一模块判定）。
  - 问五固化编号 TODO List：按优先级排序，一行一项 = 动作 + 落地路径/所需条件；无则「无」。
  - frontmatter `description` 同步补输出契约（严格六板块 / 进展 / TODO List / 验证方式与证据），skill 路由提示不受影响。
  - 全部既有合约锚点原样保留：五问标题、`## 上线结论`、completed/total、向下取整、逻辑本身/变更潜在影响、go-live ready / not ready、不把提问当侦察手段、`assumptions:`、不向用户提问；禁用字面量（当次实跑 / 复测 / Output Shape / goal: / task-plan / 跨 skill 名 / question 工具）零引入。
- **`crates/core/tests/skill_contract.rs`**：新增 `seeded_review_skill_enforces_lean_output_contract`——正向锁 `## 输出契约` / 板块之外零输出 / 严格五问 / TODO List / 编号 / 验证方式 / 证据 = `file:line` / 不写开篇引言 / 不整段粘贴 / 不跨板块复读；负向锁退役短语「没有固定输出模板」不复活。
- Seeding 传播走既有漂移覆盖机制：已安装用户的 drifted `review/SKILL.md` 会在下次 seed 时备份为 `<file>.user.bak` 并覆盖为 ship 版。

## 回归（当次实跑）

- `cargo test -p opencoder-core`：全部 12 个测试套件 ok（含 skill_contract 27 passed / 0 failed），0 failed。
- `cargo test -p opencoder-session`：**852 passed / 0 failed**（含 latent 27、skill 62 子集）。
- 因并行会话占用共享 `target/` 锁，以上在隔离 `CARGO_TARGET_DIR` 下实跑。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `seeded_review_skill_enforces_lean_output_contract`（新增） | integration | 输出契约四要点（六板块零输出 / 一行一条 / 不搬运原文 / 不跨板块复读）+ TODO List 编号 + 验证方式与 `file:line` 证据；负向锁「没有固定输出模板」已退役 |
| `seeded_review_skill_requires_five_question_recap` | integration | 五问章节 + 上线结论 + completed/total + 向下取整 + 逻辑本身/变更潜在影响 + go-live 裁决（存量锚点未破坏） |
| `seeded_review_skill_requires_no_question_tool` | integration | 不把提问当侦察手段 / assumptions: / 不向用户提问；无 task-plan token |
| `seeded_builtin_skills_carry_no_cross_skill_references` | integration | review 资产不含任何其他 skill 名 |
| `long_source_path_review_body_still_unlocks_nothing`（session） | unit | 新版 review 正文（深 HOME Source 前缀下）不解锁任何 latent 工具 |
