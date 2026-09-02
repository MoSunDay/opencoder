Commit: (working-tree, review 纯逻辑评审聚焦——不再要求实跑测试/回归;submit/summary 交叉引用对齐)

# review 纯逻辑评审聚焦（不再要求实跑测试/回归）

## 背景

- 用户要求：内置 `review` skill 不再要求执行回归测试/逻辑检测（不通过实跑命令与复测断言取证据），评审聚焦两条——**变更逻辑本身的逻辑 review** 与**变更潜在影响模块的逻辑 review**。
- 2026-09-02 早前完成的五问回退（[review-five-question-rollback-seed-drift-propagation](review-five-question-rollback-seed-drift-propagation.md)）保留了「证据须当次实跑 / 没有证据 = 没有通过」的证据纪律，与本次要求冲突，本轮收口。
- 五问结构（问一~问五 + 上线结论）与 `completed/total` 量化保持不变；变更是「验证手段」从实跑证据改为静态逻辑依据，不推翻五问回退。

## 变更

- `crates/core/assets/skills/review/SKILL.md`：问四「逐项验证+证据」→「逐项逻辑核查」——逐项回答两条：① 逻辑本身（推导/分支/边界/异常路径自洽性、与既有约定一致性，依据 = 当次 diff + 代码位置静态引用）；② 变更潜在影响（列出被改动直接/间接触及的模块：调用方、被调用方、共享类型/接口、依赖不变量，逐模块判断逻辑是否仍自洽）。Overview 新增「评审停在逻辑层」总纲（不执行测试、不跑回归、不实跑命令验证）；删除「没有证据 = 没有通过」「当次实跑」证据纪律；上线结论裁决依据改为「逻辑依据支撑」，frontmatter description 同步。五问章节名、`completed/total`+向下取整、`go-live ready | not ready` 裁决、`assumptions:` 兜底与「不调用 `question` 工具」全部保留。
- `crates/session/src/autopilot/prompts.rs::review_prompt`：合成 review prompt 对齐同一语义——评审变更自身逻辑（分支/边界/既有约定一致性）+ 触及模块（调用方/共享类型/不变量）的潜在逻辑影响；显式 `Do NOT run tests or regression suites`；保留 `Review the work completed` 前缀（`autopilot_review.rs` 转录断言依赖）。
- `crates/core/tests/skill_contract.rs::seeded_review_skill_requires_five_question_recap`：章节名与正向锁更新——「逻辑本身」+「变更潜在影响」双 token 正向锁；负向锁从「当次实跑」改为禁止「当次实跑」/「复测」回潮（skill 正文允许出现「不跑回归」的禁止性表述）。
- `agents/core/index.md` / `features/index.md`：review 职责描述行与 Commit 头同步。seeding 走 update-on-drift，ship 版变更随二进制升级自动覆盖本地漂移副本（`.user.bak` 兜底），无需改 seed.rs。

### 评审闭环（同日 review pass 裁决 not ready -> 修复翻绿）

评审（只读一次性 pass）发现主链下游对 review 输出存在旧证据语义交叉引用未闭环，按「无未闭环缺陷方为 ready」修复后翻绿：

- `crates/core/assets/skills/submit/SKILL.md`（P2）：衔接段「消费 `review` 的当次证据汇总」→「逐项逻辑核查与影响面汇总」；gate 已绿前提注明由 do-and-done 的实现验证 + rules/02 迭代回归 gate 保证（review 只产出逻辑评审，不再产出实跑证据）。
- `crates/core/assets/skills/summary/SKILL.md`（P3）：「不替代 review 的证据评审」→「逻辑评审」。
- `crates/session/src/autopilot/review_pass.rs`（P3）：注释澄清 `review_prompt` 的 "do NOT run tests" 为指令性约束而非工具门禁（act agent 仍有 bash），只读边界靠 prompt + review skill 契约维持。
- `crates/core/tests/skill_contract.rs`：新增 `seeded_submit_skill_consumes_review_logic_recap`——正向锁「逐项逻辑核查与影响面汇总」+ 负向锁（不得回潮「证据汇总」）+ gate 前提锚（go-live ready / rules/02 / 迭代回归）。
- `do-and-done/SKILL.md:35` 的「最终证据汇总」为其自身实跑验证的产出，与 review 输出无耦合，不属本次闭环范围（保留）。

## 回归（当次实跑）

- `cargo test -p opencoder-core --test skill_contract`：**24 passed / 0 failed**（含更新后的五问合约锁 + 新增 submit 交叉引用锁）。
- `cargo test -p opencoder-session --test autopilot_review`：**6 passed / 0 failed**（review pass 全链路，含合成 prompt 转录断言）。
- 全量回归：`cargo test --workspace --no-fail-fast` → 248 个测试二进制 **3882 passed / 0 failed**（EXIT=0）；`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 过程注记：中途两轮全量曾出现 `skill_mid_run` / `skill_tail_cleared_after_run_end` 瞬时失败——均为工作区内并行任务的 skill 投递重构 WIP（`skill_context.rs`/`skill_lifecycle.rs`/`runner/llm_call.rs`，非本任务改动面）的中间态，待其收敛后复跑即绿，最终全量全绿；与本任务改动（资产文本 + 注释 + core 测试）无交集。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `seeded_review_skill_requires_five_question_recap` | integration | 五章节（问四改「逐项逻辑核查」）+ completed/total/向下取整 + 逻辑本身/变更潜在影响正向锁 + 负向锁（无「当次实跑」「复测」）+ go-live 裁决 + 无 Output Shape/goal: 模板残留 |
| `seeded_review_skill_requires_no_question_tool` | integration | 不把提问当侦察手段 + assumptions: + 不调用 question 工具；负向锁（无「可在同一轮多问」、无 task-plan token）保留 |
| `seeded_submit_skill_consumes_review_logic_recap` | integration | submit 消费 review 的「逐项逻辑核查与影响面汇总」；负向锁无「证据汇总」回潮；gate 前提锚定 do-and-done 验证 + rules/02 迭代回归 |
| `review_mode_runs_exactly_one_review_pass` 等 6 例 | integration | review pass 机制回归：单轮 review turn、skill 激活/清除、plan 模式可跑、错误路径清 skill——合成 prompt 改写后语义不回归 |


