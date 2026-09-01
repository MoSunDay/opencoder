Commit: (working-tree)

# plan 模式 `--prompt-file` 不再注入 build 委派广告；hide 判定收敛为 core 单一谓词

## 背景

审计发现：`opencoder run --agent plan --prompt-file f` 用 `body + tool_preamble()`
**整体替换** `session.agent.prompt`，而 `tool_preamble()` 无条件含
`", 'build' (full tools) for implementation"`。下游 `build_system` 只在
task-plan skill 激活时剥除——plan kind 的自定义提示词路径把 build 广告
泄漏进模型可见系统提示词。仅提示词层「知情」泄漏：schema（`hide_build_subagent`
恒 true for Plan）与运行时（`plan_subagent_guard`）由 `agent.kind` 驱动，
写能力无逃逸。默认路径（内置 `base_prompt_plan` 构造期剥除）一直成立。

## 实现

- **`crates/core/src/agent.rs`**：新增单一谓词
  `build_delegation_hidden(kind, task_plan_active) -> bool`
  （Plan 恒 true，其余 mode 在 task-plan 激活时 true）——prompt 剥除与
  schema 隐藏的唯一事实源，消除三处（base_prompt_plan / build_system /
  hide_build_subagent）各自漂移的可能。`lib.rs` 根导出该谓词与
  `BUILD_DELEGATION_CLAUSE`。
- **`crates/session/src/prompt.rs::build_system`**：剥除条件从
  `task_plan_active(skill)` 升级为共享谓词（kind 感知）——所有自定义提示词
  路径在模型可见装配点兜底；内置 plan 提示词处仍为 no-op（构造期已剥）。
- **`crates/session/src/tools/mod.rs::hide_build_subagent`**：改为对 core
  谓词的薄投影（语义不变，矩阵测试原样通过）；文档措辞修正
  （"sandbox mode" → "plan mode（曾以 sandbox 序列化）"）。
- **`crates/cli/src/run.rs`**：`--prompt-file` 合成提为纯函数
  `compose_custom_prompt(kind, body)`——plan kind 时只对注入的
  `tool_preamble` 剥除 build 子句（**不动用户正文**），存储态即干净；
  skill 激活剥除仍由 `build_system` 按 turn 复查（合成时刻无 skill 可激活）。

## 边界与权衡

- 剥除只作用于系统注入的 preamble，用户 `--prompt-file` 正文原样保留
  （系统不消毒用户内容）。
- act + prompt-file 行为逐字节不变（`compose_custom_prompt` 对 Act 原样
  拼接），非 regression 面。
- `session show --json` 观测面与存储态提示词不再携带广告子句。

## 测试覆盖

| 层面 | 测试名 | 文件 |
|------|--------|------|
| core 谓词矩阵 | `build_delegation_hidden_matrix` | `crates/core/src/agent.rs` |
| preamble 是剥除靶子（含负向） | `tool_preamble_build_clause_is_strip_target` | `crates/core/src/agent.rs` |
| plan 自定义 prompt 装配后无广告 | `build_system_strips_build_clause_for_plan_custom_prompt` | `crates/session/src/prompt.rs` |
| act 自定义 prompt 保留完整 preamble | `build_system_keeps_preamble_for_act_custom_prompt` | `crates/session/src/prompt.rs` |
| `--prompt-file` plan 合成无 build 字样 | `prompt_file_plan_composition_omits_build_delegation` | `crates/cli/src/run.rs` |
| `--prompt-file` act 合成不受影响 | `prompt_file_act_composition_keeps_full_preamble` | `crates/cli/src/run.rs` |
| `run()`→`session.agent.prompt` 赋值接缝（mock 端到端） | `run_with_plan_agent_prompt_file_stores_prompt_without_build_ad` / `run_with_act_agent_prompt_file_stores_full_preamble` | `crates/cli/tests/prompt_file_run_assignment.rs` |
| 运行时守卫（回归，提示词无关层） | `plan_subagent_guard`（3 tests） | `crates/session/tests/plan_subagent_guard.rs` |
| schema 隐藏矩阵（回归） | `hide_build_subagent_matrix` | `crates/session/src/tools/mod.rs` |
| 内置 plan prompt 构造期剥除（回归） | `plan_prompt_strips_build_subagent_advertisement` | `crates/core/src/agent.rs` |

## 全量回归

- `cargo test -p opencoder-core` / `-p opencoder-session`（lib 423+ +
  `plan_subagent_guard` 3 passed）/ `-p opencoder-cli`（lib 86 + 集成全套）
  → 全绿（当次树采集）。
- `cargo test --workspace --no-fail-fast` → 132 个目标全绿；唯一红
  `nodes_smoke_proc` 为环境性：daemon server 冷启动 107s 超过 smoke 脚本
  90s 就绪预算（compile storm 下），手工复刻同命令 server 107s 可达、
  `/api/time` 正常响应，消歧转绿；并行门禁独立读数 3855/1 同此归因。
- clippy：`-D warnings` 全 workspace 零警告（当次树）。
- build：`cargo build --workspace` 干净。

## Related Docs

- [agents/core](../../../agents/core/index.md)
- [agents/session](../../../agents/session/index.md)
