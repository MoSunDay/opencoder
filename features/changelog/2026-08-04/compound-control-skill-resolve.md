Commit: (working-tree, pre-initial-commit)

# 复合控制命令解析 + `$skill` token 在所有 delivery 路径一致激活

## 背景
`/plan review` 这类复合命令（控制前缀 + 剩余文本）此前仅在 idle 路径能
正确切换模式并运行剩余文本，queue/steer 路径缺失。此外 `$skill` 内联 token
解析只发生在 compound 命令路径中，普通排队/steer prompt（如 `$review do it`）
会将 `$review` 原样泄漏给 LLM。纯 skill 复合输入（`/plan $review` 无其他文本）
在 queue/steer 中还会记录空 user message 并误增 `plan_input_count`。

## 变更
### 复合命令解析（split_control_prefix）
- **`crates/session/src/control_cmd.rs`**：新增纯函数
  `split_control_prefix(prompt) -> Option<(ControlCmd, Option<String>)`，
  将控制前缀与剩余文本拆分；`parse` 委托给它保持向后兼容。
  `/act_clear_context` 为 sentinel，仅精确匹配、不接受参数。

### `$skill` token 解析（skill_resolve 模块）
- **`crates/session/src/skill_resolve.rs`**（新建，225 行）：
  `resolve_inline_skills_with` / `resolve_inline_skills` 扫描 `$name`
  token，查 skill 注册表并激活 skill body（`unlocked_from_body` 自动解锁
  latent tools），未解析 token 原样保留。`record_compound` 统一负责
  queue/steer 路径的 prompt 记录：解析 token → 应用 plan-mode tag → 持久化；
  纯 skill 输入（token 耗尽后文本为空且无图片）注入 `SKILL_TRIGGER`
  而非空消息，跳过 `plan_input_count` 递增，与 idle 路径一致。

### 三条 delivery 路径整合（runner/mod.rs）
- **`crates/session/src/runner/mod.rs`**（794 行）：
  - idle L75：compound 剩余文本不再提前 resolve，改在记录块统一处理，
    并对**所有** headless prompt（compound + plain）执行 `resolve_inline_skills`。
  - steer L226：compound 命令走 `record_compound`；**普通 steer prompt**
    也改走 `record_compound`，使 `$review analyze this` 能正确解析。
  - queue L349：普通 queue prompt 同样改走 `record_compound`。
  - idle 触发消息改用共享常量 `SKILL_TRIGGER`，消除重复字符串。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| split_control_prefix 拆分 | 9 个 split_* unit tests | control_cmd.rs |
| record_compound 干净文本 | record_compound_records_cleaned_text | skill_resolve.rs |
| record_compound 纯 skill 触发 | record_compound_pure_skill_injects_trigger | skill_resolve.rs |
| record_compound 空无 skill | record_compound_empty_no_skill_records_nothing | skill_resolve.rs |
| resolve_inline_skills 7 项 | resolves/dedupes/mixed/unresolved 等 | skill_resolve.rs |
| idle compound /plan review | idle_compound_plan_arg_switches_then_runs | tests/control_cmd.rs |
| queue compound /plan review | queue_compound_plan_arg_switches_then_runs | tests/control_cmd.rs |
| compound $review 激活 skill | compound_plan_with_dollar_activates_skill | tests/control_cmd.rs |
| queue 纯 skill 注入触发 | queue_compound_pure_skill_injects_trigger | tests/control_cmd.rs |
| queue 普通 $skill 解析 | queue_plain_skill_prompt_resolves | tests/control_cmd.rs |
| steer 普通 $skill 解析 | steer_plain_skill_prompt_resolves | tests/control_cmd.rs |

- 全量回归：`cargo test -p opencoder-session` → 268 unit + 13 集成 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- 行数：skill_resolve.rs 225 ≤ 400；runner/mod.rs 794 ≤ 800

## Impact Surface
- 用户：`/plan review`、`/act do it` 在 queue/steer/idle 任何路径都能
  切换模式并运行剩余文本；`$skill` token 在所有路径一致解析。
- 不影响：Store trait、LLM 后端、Web SSE 协议、CLI 命令结构。

## Related Docs
- [agents/session](../../agents/session/index.md)
