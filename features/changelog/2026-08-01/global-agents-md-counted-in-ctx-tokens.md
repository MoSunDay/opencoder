# 全局 agents.md 重新计入 ctx token（撤销 2026-07-18 的排除）

## 背景

2026-07-18 起，全局 `~/.opencoder/AGENTS.md` 的 token 被从两处 ctx 统计中排除：
TUI 上下文计量条（`app_helpers.rs::sys_tokens_for`）与压缩预算（`compaction.rs::estimated_tokens` / `reported_tokens`），理由是「常驻基线上下文不应占用单会话对话预算」。

实测口径对不上：act 模式下系统提示（`base_prompt_act` + 本仓 `agents.md` + 环境块）当前为
**898 token**（仓库自带估算器 `chars/4`），而全局约束文件 `~/.opencoder/AGENTS.MD`
（约 157 token）**并未算进这个数**——898 恰是「不含全局」的口径。全局文件仍随系统提示
发给模型（`build_system` 未改），模型真实消耗这些 token，因此预算与计量条应与实际
请求一致，把全局约束计入。

## 变更

### 三处计费点不再扣除全局 token

- `compaction.rs::estimated_tokens`（估算信号）：删除对 `global_instructions_text` 的
  `saturating_sub`。系统提示经 `build_system` 已含全局文件，估算自然覆盖。
- `compaction.rs::reported_tokens`（模型上报的真实 input_tokens）：直接返回
  `last_usage.input_tokens`，不再扣减全局 token——上报值本就含系统提示。
- `app_helpers.rs::sys_tokens_for`（TUI 计量条）：直接返回 `estimate(&text)`，不再扣减。
  启动即反映真实系统提示大小（act 模式 898 → 1057，+全局 157 token）。

全局内容**仍照常**随系统提示发送（`prompt.rs::load_instructions` 未改），只是口径恢复为
「发多少、算多少」。

### 删除已无用途的 `global_instructions_text`

该函数（`crates/session/src/prompt.rs`）唯一用途就是给上述三处扣减提供全局文本；
排除撤销后成为死代码，连同其 4 条单测一并删除。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 全局 agents.md 计入压缩预算（差分：有文件必触发、无文件不触发） | `global_agents_md_counts_toward_compaction_budget` | `session/tests/compaction_and_model.rs` |
| 系统提示 token 计入计量条（确定性 + skill 增量 + 未知 agent=0） | `sys_tokens_counts_system_prompt` | `tui/src/app_tests/skill_tests.rs` |
| 已删除：`global_instructions_*` 4 条单测（函数随功能撤销） | — | `session/tests/prompt.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 96 个测试二进制全部 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |

## Impact Surface

- 用户可感知：TUI 计量条 / 压缩触发点现在包含全局 agents.md 的 token（大全局文件会
  更早触发压缩——恢复 2026-07-18 之前的口径）。
- 系统提示内容与发送路径不变；`load_instructions`（全局 + git-root + working-dir 合并）
  不受影响。
- 溢出安全性：默认 `context_limit=128_000`、预算 80k，仍留 48k 余量；`reported_tokens`
  以真实上报值计，`reserved` 保留余量。

## Related Docs

- 撤销前序：[global-agents-md-excluded-from-ctx-tokens](../../2026-07-18/global-agents-md-excluded-from-ctx-tokens.md)
- [agents/session](../../agents/session/index.md) — 压缩预算
- [agents/tui](../../agents/tui/index.md) — 上下文计量条
