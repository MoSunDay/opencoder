# 新增内置 skill：say-and-replay（复述与回放 / 对齐快照）

## Context

工作流主链 `task-plan → do-and-done → review → submit` 与正交的 `summary`（回顾）已具备，
但缺少一个**随时对齐进度、暴露阻塞、支撑交接/恢复**的只读快照工具。需要一个 skill：在任意
检查点把当前任务「说清楚原始需求目标本身 → 回放已做的 TODO 及验收证据 → 暴露当前卡点 →
列出后续完全闭环所需 TODO」一气呈现。

`say-and-replay` 即此工具：正交于主链、与 `summary` 平级，是只读的「对齐快照」，不修改代码、
不提交、不重排 STATUS 块。技术上是纯 markdown skill（无 Rust 逻辑、无 latent tool），仅需
新增 asset + 在 `BUILTIN_SKILLS` 注册 + 更新契约测试与文档。

## Change Summary

### 新增 skill asset
- **`crates/core/assets/skills/say-and-replay/SKILL.md`**（新增，63 行）：frontmatter
  `name: say-and-replay` + 英文 `description`；正文中文，结构对齐 `summary`/`review`：
  `## 角色`（带 `> **只读**`）→ `## 何时使用` → `## 输入` → `## REPLAY 块（固定输出格式）`
  → `## 原则` → `## 与其它 skill 的衔接`。
- **REPLAY 块**字段精确映射诉求：
  - `goal:` 复述**原始**需求目标 + 验收标准（来自用户原始 prompt，非派生 STATUS goal）
  - `progress:` completed/total（仅计入有证据项）
  - `done:` 已完成 TODO + `accept:` + `evidence:`
  - `doing:` 进行中 / 证据不足项 + `reason:`
  - `blocked:` 当前卡点 + `blocks:`（阻塞了谁）+ `need:`（解除需要什么 / 等谁）
  - `remaining:` 后续闭环 pending TODO + `accept:` + `impact:`
  - `verdict:` `on-track | at-risk | blocked | done`

### 注册内置 skill
- **`crates/core/src/skill.rs`**：
  - `BUILTIN_SKILLS` 新增 `say-and-replay` 项（置于 `review` 与 `summary` 之间），
    `include_str!` 内嵌 `SKILL.md`。`seed_builtin_skills_in` 泛型遍历自动接纳，零行为分歧。
  - 更新 `BUILTIN_SKILLS` 上方文档注释，列举 `say-and-replay`。
  - **未**在 `skill.rs` 内加 inline 测试：该文件已达 776 行（HEAD），加内联测试会越过
    800 行迭代上限，故回归覆盖下沉到集成层 `tests/skill_contract.rs`（见下，更符合
    rules/03 分层：用 pub seed/discover API + tempdir）。

### 契约测试与文档
- **`crates/core/tests/skill_contract.rs`**：`seed_in_writes_all_packs_on_fresh_dir` 期望
  数组加入 `"say-and-replay"`（fresh dir discover 出该 skill）。
- **`features/index.md`**：内置 skill 清单与主链说明补上 `say-and-replay`（标注为正交
  只读对齐工具，与 `summary` 并列、不在主链）。
- **`agents/core/index.md`**（repair-on-touch）：`Skill` 段内置 skill 枚举补上
  `say-and-replay` 并在「正交工具」句中并列。

## Validation

> 注：当前工作区存在**先于本任务、范围外的未完成 MCP 集成 WIP**（`crates/session`/`crates/tui`/
> `crates/core/src/config/mcp.rs`），使 `opencoder-tui` 无法编译（`build_system` 形参不匹配、
> `SlashAction::Mcp` 未覆盖）。该 WIP 非本任务引入，不计入本变更。故 go-live 验证以**隔离后**
> （仅本变更、MCP WIP 暂存）的 workspace 构建与 `opencoder-core` 全套为准。

- `cargo build -p opencoder-core` → Finished，零错误零警告。
- `cargo clippy -p opencoder-core --all-targets -- -D warnings` → Finished，零警告。
- `cargo test -p opencoder-core` → 全绿：lib `91 passed`、`skill_contract 13 passed`、
  `config_contract 29`、`message_image 11`、`tool_filter 16`、`tool_output_image 5`，
  合计 `165 passed; 0 failed`。
- 隔离验证（仅本变更）：`cargo build --workspace` → Finished。
- 隔离 `cargo test --workspace`：除一个**环境相关 flaky** 测试 `bash_failure_appends_exit_code`
  （沙箱中 `exit 7` 回传 `-1`；该测试在 pristine HEAD 单跑通过、与本变更无关）外，全绿。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| seeded say-and-replay frontmatter name/description/body REPLAY 块完整（防 include_str! 路径写错 / 空注册） | `seeded_say_and_replay_skill_is_well_formed` | `crates/core/tests/skill_contract.rs` |
| fresh dir 上 discover 出 say-and-replay（含全部内置 skill） | `seed_in_writes_all_packs_on_fresh_dir` | `crates/core/tests/skill_contract.rs` |

## Related Docs

- [agents/core](../../../agents/core/index.md)（`Skill` 内置 skill 枚举段）
- [features/index.md](../../index.md)（Skill 选择 / 内置 skill 清单）
