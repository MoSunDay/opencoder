# 内置 skill 自包含收敛——清除跨 skill 指涉与 question 越权文档

Commit: (working-tree, skill self-contained 收敛)

## 背景

- 工作树新增契约（`crates/core/tests/skill_contract.rs`，用户落笔）：内置 skill 必须 self-contained——任何 skill 资产不得携带其它内置 skill 名（跨 skill 指涉既把 plan→execute→review→submit 编排错误地编码进 skill 正文，`task-plan` 字样还会劫持 latent `question` 解锁的 500 字符窗口）；`question` 工具文档只允许存在于 task-plan 一处。
- 旧的「排版重整（内容零变更）」路线随之废弃：保留交叉指涉的重排版已被全量回滚，本轮按新契约做最小内容收敛，不做格式重排。

## 变更

- `review/SKILL.md`：锚点句 `不调用 \`question\` 工具提问` → `不向用户提问`（与契约测试 guidance 换词对齐），其余零改动。
- `do-and-done/SKILL.md`：description 去掉 `produced by task-plan`（改 `produced by the planning phase`）；正文 3 处 `task-plan` → 规划阶段/闭环计划；暂停协议 `question` 工具表述改为「可向用户提问（交互式 TUI 澄清通道）」；整节删除「## 与 task-plan 的衔接」（编排职责归 caller/system prompt）。
- `say-and-replay/SKILL.md`：只读边界与原则中的 4 个 skill 名改为角色词（实现循环/规划/提交流程/任务回顾）；STATUS 块数据源表述去 `task-plan`；整节删除「## 与其它 skill 的衔接」（5 条）。
- `submit/SKILL.md`：角色/前置 gate/changelog 生成/不可逆暂停协议各节保留，其中 `review`→评审、`do-and-done`→实现循环、`repo-local-memory`→记忆维护规范 换词；衔接节改写不删（`task-plan`→闭环计划），锚点 `逐项逻辑核查与影响面汇总`、`go-live ready`、`rules/02`、`迭代回归` 原样保留（锚点在节内）。
- `summary/SKILL.md`：5 处 skill 名改角色词，衔接节 3 条改写为不点名职责描述。
- `repo-local-memory/SKILL.md`：`- a PR summary` → `- a PR recap`（子串级契约误伤修正）。
- `repo-local-dreaming/SKILL.md`：4 处 `repo-local-memory`/`summary`/`submit` 改为「迭代内记忆维护/任务级回顾/提交流程」。
- 未动：`task-plan/SKILL.md`（question 唯一文档面）、`task-plan/references/launch-closure-plan-checklist.md`、`ssh-pty`、`chrome-headless`（本就零交叉指涉）。

## Validation（当次实跑）

- `cargo test --workspace --no-fail-fast`：全绿（247 套件 0 failed，含 `skill_contract` 26/26——首轮曾因 review「证据纪律」节与 submit 前置节被整节误删而 2 failed，已按「锚点保留、换词收敛」意图恢复后复跑全绿）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo fmt --all --check`：干净。
- 锚点全绿：review（五问标题/上线结论/`不向用户提问` 等正负向）、do-and-done（闭环计划/go-live ready/无 STATUS 块）、submit（逐项逻辑核查与影响面汇总/rules/02/迭代回归/无证据汇总）、summary（无 STATUS 块）、say-and-replay（六字段/百分比/（<0-100>%）、task-plan（澄清协议等全组 + 注入前缀含 task-plan 与 question）、checklist（## 1.-## 4. + Plan Output Schema）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| skill 零交叉指涉（self-contained） | `seeded_builtin_skills_carry_no_cross_skill_references` | crates/core/tests/skill_contract.rs |
| question 文档仅限 task-plan | `seeded_question_tool_docs_live_only_in_task_plan` | crates/core/tests/skill_contract.rs |
| review 不提问锚点（换词） | `seeded_review_skill_requires_no_question_tool` | crates/core/tests/skill_contract.rs |
| 消费闭环计划 + STATUS 负向 | `seeded_workflow_skills_consume_launch_closure_plan` | crates/core/tests/skill_contract.rs |
| submit 消费逻辑汇总 | `seeded_submit_skill_consumes_review_logic_recap` | crates/core/tests/skill_contract.rs |
| say-and-replay REPLAY 字段锁 | `seeded_say_and_replay_skill_requires_five_question_recap` | crates/core/tests/skill_contract.rs |
