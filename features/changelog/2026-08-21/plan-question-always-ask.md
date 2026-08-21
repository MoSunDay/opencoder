# plan 模式提问行为修复：ask-by-default（有疑必问，先查再问）

## 背景

plan 模式下模型从「第一次会提问、之后不再问」退化为基本不问。排查确认机械链路
（question 工具注册、schema 注入、hub attach、事件泵、question_menu 状态机）全部正常，
根因在提示词面三处合力：

1. **回归（实锤）**：`04df804`（08-19）重写 task-plan SKILL.md 时删光「澄清协议
   （question 工具指引）」小节，并连带删除守卫测试
   `seeded_task_plan_skill_requires_question_tool_guidance`。
2. **措辞过保守**：`PLAN_SUFFIX` 与 question 工具 description 双重
   "genuinely ambiguous / Use ONLY when" 门——模型默认把需求判为「够清楚」而不问。
3. **尾缀 tag 副作用**：plan 阶段第 2 条起的 prompt 尾缀
   「（当前处于只读的 plan 模式，聚焦计划生成）」字面净效果是"别来回交互、直接出计划"，
   造成同阶段第 1 条会问、后续每条都不问的确定性差异。

## 变更

- **`crates/core/assets/skills/task-plan/SKILL.md`**：恢复压缩版
  「## 澄清协议（存在影响计划的疑问时）」小节，适配 04df804 后的上线闭环新结构：
  会改变计划走向的未定事项必先对齐（无疑问不强制问）／先查再问（仓库、`rules/`、
  既有测试、`AGENTS.md` 可查的事实不把提问当侦察）／`question` 可用（plan agent +
  交互式 TUI）时逐句多轮澄清／不可用（headless/web）时显式假设列入 `assumptions:`
  继续规划。与 Workflow 第 1 步「强卡点必须问」互补不冲突。
- **`crates/core/src/agent.rs`**：`PLAN_SUFFIX` 第三句改为 ask-by-default——
  finalizing 前必须经 `question` 澄清每一个会影响计划的未定项，任何依赖用户未说明
  且仓库推不出的假设都先问（可一轮多问），仓库/rules/测试可查的事实先查不问；
  tools allowlist 注释同步。**`BASE_PROMPT`（act 模式提示词）零改动。**
- **`crates/session/src/tools/question.rs`**：description 从
  "Use ONLY when … genuinely ambiguous" 放宽为
  "Prefer asking over assuming whenever an unstated assumption would shape the plan;
  look up repo/rules/test facts first instead of asking"，保留先查仓库防侦察守卫；
  模块 doc 同步。
- **`crates/session/src/lib.rs`**：`maybe_tag_plan_prompt` 尾缀 tag 追加子句
  「存在影响计划的疑问必须先用 question 工具提问再输出计划」，消除「聚焦计划生成」
  对提问行为的抑制；doc 注释同步。仅 `AgentKind::Plan` 分支，act 模式永不追加。
- **`features/index.md`**：plan 结构化提问条目追加本次 changelog 链接。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| task-plan 澄清协议恢复（回归守卫） | `seeded_task_plan_skill_requires_question_tool_guidance` | `core/tests/skill_contract.rs` |
| task-plan 上线闭环主线未破坏 | `seeded_task_plan_skill_requires_launch_closure_contract` | `core/tests/skill_contract.rs` |
| 删除旧 SKILL.md 后 re-seed 恢复澄清协议（运行手册守卫） | `deleted_task_plan_skill_is_reseeded_with_clarification_protocol` | `core/src/skill/seed.rs` |
| PLAN_SUFFIX ask-by-default 措辞 + 可一轮多问 | `plan_prompt_allows_multiple_questions_per_turn` | `core/src/agent.rs` |
| BASE_PROMPT 剥离逻辑未变（act 面不受影响） | `plan_prompt_strips_build_subagent_advertisement` | `core/src/agent.rs` |
| question 工具仍 plan-only（act/explore/build 不注入 schema） | `question_tool_is_plan_agent_only` | `core/src/agent.rs` |
| question description 新门 + 反侦察守卫 | `description_allows_several_questions_per_turn` | `session/src/tools/question.rs` |
| 尾缀 tag 新子句（第 2 条起生效） | `plan_second_prompt_tagged` | `session/src/lib_tests.rs` |
| act 模式永不追加 tag | `act_mode_never_tagged` | `session/src/lib_tests.rs` |
| 首条 prompt 不加 tag | `plan_first_prompt_not_tagged` | `session/src/lib_tests.rs` |
| handoff 后计数重置 | `switch_to_plan_resets_count` | `session/src/lib_tests.rs` |
| tag 三注入点端到端（direct/steer/queue） | `direct_prompt_tags_only_after_first` / `steer_prompt_tagged_after_first` / `queued_prompt_tagged_after_first` | `session/tests/plan_tag.rs` |

- 变更面全量回归：`cargo test -p opencoder-core -p opencoder-session` →
  **1046 passed / 0 failed**
- clippy：`cargo clippy -p opencoder-core -p opencoder-session --all-targets -- -D warnings`
  → 零警告
- 全仓 gate 收口（并行工作流落地后补跑）：`cargo test --workspace` →
  **3245 passed / 0 failed**（212 个测试二进制；首轮曾见 web 的
  `interrupt_beats_pending_replay` 单次失败，复跑全量与单测 5/5 均绿，判定并发抖动，
  属并行工作流测试面）；`cargo clippy --workspace --all-targets -- -D warnings`
  → 零警告。
- 行为验证说明：本修复是提示词语义，MockChatClient 无法覆盖；需真模型下连续第 2、3
  条 plan prompt 观察 question 弹框。

## 风险

- **提问频率上升**：ask-by-default 可能多问——保留「先查再问／不把提问当侦察」守卫
  平衡；headless/web 无监听即刻回兜底文案，不受影响。
- **seed never-clobber**：已安装机器上 `~/.opencoder/skills/task-plan/SKILL.md`
  不会自动更新，需手动删除该文件后重启 re-seed（与 ac250e5 先例一致）。
- **全仓 gate 快照**：本轮改动期间，工作树内另有一条并行在途工作流（web/cli/client
  的 question 对齐特性）持续编辑 `src/main.rs`、`crates/web/*`、`crates/cli/*` 等，
  workspace 整体编译随其推进波动（曾缺 `mod handle_questions;` 声明与测试导入，
  已做两处最小解阻修复：`web/src/lib.rs` 补模块声明、`core/src/data_dir.rs` 测试补
  `use std::path::Path`）；快照时 `cargo build --workspace` 仍因 `src/main.rs:104`
  E0308（`ClientSub` 未实现 `Clone`）失败，属该工作流未完成代码，与本变更集无关。
  **后记（gate 收口）**：该工作流落地后上述阻塞解除，全仓 build/test/clippy 已全绿
  （数字见「回归」段），本风险项关闭。
