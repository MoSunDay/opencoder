Commit: (working-tree, task-plan 去 Any Home + question 收敛 task-plan 专属解锁)

# task-plan 去 Any Home 协议；question 工具取消默认注入

## 背景

两处收敛：

1. 内置 `task-plan` skill 残留整套 Any Home 规划协议（PlanRun/SupportRequest/QueryRecord、`scripts/any_home_planning.py` 写回流程），但仓库并不携带该脚本，纯属死指令，白白占据 skill body 与模型注意力。
2. `question` 工具在 sandbox agent 上无条件豁免 latent 门控（`is_visible` 对 Sandbox 直接放行），且 `SANDBOX_SUFFIX` 在系统提示词里直接描述其用法——而 question 只有执行 task-plan 时才用得上，默认注入浪费 token 与注意力；工具的用法描述应随 skill 走，而不是常驻系统提示词。

## 实现

- **task-plan 资产瘦身**：`SKILL.md` 删除 Any Home 相关 5 行（建立规划上下文 2 条、产出闭环执行规划 PlanRun 2 条、Notes 1 条）；`references/any-home-plan-run.md` 删除；`skill/seed.rs` 内嵌表移除对应条目；已 seed 的用户副本（`~/.opencoder/skills/task-plan/`）同步删除（该副本本轮与内置逐字节一致，按用户要求同步收敛；seeding 本身仍 never-clobber，不会自动覆盖用户手改）。
- **question 统一 task-plan 门控**：`latent.rs::is_visible` 删除 sandbox 豁免——`question` 对所有 agent 一视同仁，均需 task-plan skill body 前 500 字符解锁；sandbox 的 `ToolFilter::Allow` 保留 `question`（allowlist 只管资格，可见性由 latent 门控决定），`SANDBOX_SUFFIX` 删除 question 澄清协议句——该协议描述现在只存在于 task-plan skill body（澄清协议一节措辞同步改为「由本 skill 解锁，act / sandbox 一视同仁，无 skill 时不注入」，且 `question` 字样仍处于前 500 字符解锁窗内，契约测试守护）。
- **测试面重写**：`question_gating.rs`（sandbox 无 skill 隐藏 / 有 task-plan 可见双测）、`tools/mod.rs::question_schema_is_task_plan_gated_and_compact`（成本基线改由 act+plan body 度量，schema 仍 <200 tokens）、`latent.rs`/`latent_tools.rs`（`visibility_sandbox_needs_skill_unlock_too`）、`agent.rs`（sandbox prompt 不再出现 question 字样的结构守护）、`skill_contract.rs`（退役 reference 不得回种 fresh install；用户既有副本 never-clobber 存活）。
- **记忆修复**：`agents/core/index.md`（sandbox tools 描述、SANDBOX_SUFFIX 语义、references 描述）、`agents/session/index.md`（latent 门控段、question_gating 测试清单描述）。

## 测试

- `cargo test -p opencoder-core` → 314 passed / 0 failed（含 skill_contract：seed 表、退役协议不回种、前 500 字符解锁窗）
- `cargo test -p opencoder-session` → 746 passed / 0 failed（含 question_gating 重写双测、latent_tools、tools 估算器）
- 全量回归 `cargo test --workspace` → 3345 passed / 0 failed（当次实跑，TEST-EXIT=0）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **0 告警**（CLIPPY-EXIT=0）。〔终验补录：前记「并发工作流阻塞 shellguard 未收口」前置已在 shellguard 换壳收口提交后解除，提交前实跑复验。〕
