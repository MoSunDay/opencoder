Commit: (working-tree, post-1ba8f426)

# 移除内置 `fk-cli` skill

- 内置 skill 表（`BUILTIN_SKILLS`）移除 `fk-cli`：资产目录 `crates/core/assets/skills/fk-cli/` 与 seed 条目一并删除，新装不再随附该 skill。
- 已 seed 到 `~/.opencoder/skills/fk-cli` 的存量副本不受影响：seeding 保持 never-clobber / never-delete 语义，不覆盖、不删除用户目录中的既有文件。
- `fk-session` 聚焦移动 UI 执行合同如仍需要，可由用户以用户级 skill 自行放置到 `~/.opencoder/skills/`。
- 注册 CLI 注入机制（`Config.cli` → system prompt `Registered CLI` 段）不受影响；仅测试 fixture 键名由 `fk-cli` 改为中性 `test-cli`。

## 变更文件
- `crates/core/assets/skills/fk-cli/SKILL.md`：删除。
- `crates/core/src/skill/seed.rs`：`BUILTIN_SKILLS` 表删除 `fk-cli` 条目；doc comment 枚举同步去掉该项。
- `crates/core/tests/skill_contract.rs`：fresh-dir seed 期望名单去掉 `"fk-cli"`。
- `crates/session/src/runner/llm_call.rs`：两处 CLI 注入测试 fixture 键名 `fk-cli` → `test-cli`。
- 记忆文档同步（agents/core、agents/todos、features/todos）：清理 `fk-cli` / `$fk-cli` 过期引用；`features/changelog/2026-08-15/` 历史条目不动（append-only）。

## 测试清单
- `crates/core/tests/skill_contract.rs::seed_in_writes_all_packs_on_fresh_dir`：期望内置包集合不再含 `fk-cli`，并新增缺席断言 `fk-cli` 不再被 seed。
- `crates/session/src/runner/llm_call.rs` CLI 注入测试（`cli_injected_only_into_explore_subagent_by_name` / `mcp_tools_hidden_from_workflow_agent`）：fixture 键名改为 `test-cli`，注入语义断言不变。
