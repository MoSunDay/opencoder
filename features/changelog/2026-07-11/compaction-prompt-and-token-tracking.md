Commit: (working-tree, pre-initial-commit)

# 压缩提示词改为 opencode 结构化模板 + token 追踪对齐 + 死配置清理

## 背景
压缩摘要提示词为一句话自由格式，摘要质量不稳定、结构不可预测。
`reported_tokens` 用 `total_tokens`（input+output）对比输入预算，输出大的 turn 会误触发压缩。
TUI 状态栏 `track_context` 忽略 `ToolStart` 的 input JSON，导致状态栏 ctx% 与 `should_compact` 的估算基准不一致。
`CompactionConfig.prune` 字段声明、解析、显示但从不被读取——死配置。

## 变更

### A — 压缩提示词改为 opencode 结构化模板（`prompt.rs` + `compaction.rs`）
- **新增 `compaction_system_prompt()`**：固定 anchored-context-summarization assistant 角色，指示增量更新 `<previous-summary>`、严格按模板输出、不提及自己在做摘要。
- **新增 `compaction_user_prompt(previous_summary: Option<&str>)`**：结构化 Markdown 模板——Objective / Important Details / Work State (Completed/Active/Blocked) / Next Move / Relevant Files 六段，末尾 Rules 要求保留段落、terse bullets、保留精确路径/命令/标识符。有 `previous_summary` 时插入 `<previous-summary>` 块并切指令为"更新"，否则"创建新摘要"。
- **`summarize()` 注入 system message**：消息序列从 `[head] ++ [user_prompt]` 变为 `[system_prompt] ++ [head] ++ [user_prompt]`。
- **`compact()` 提取旧 summary**：在 head 中查找既有 synthetic user summary（`[Conversation summary so far]` 前缀），剥前缀后传入 `summarize()` 作为 `previous_summary`，实现**增量更新**而非每次重写。

### B — `reported_tokens` 改用 `input_tokens`（`compaction.rs`）
- 从 `total_tokens`（input + output）改为 **`input_tokens`**。
- 修复：输出大的 turn（如 8k output）不再把 `total_tokens` 推过输入预算而误触发压缩。

### C — TUI `track_context` 对齐（`app.rs`）
- `track_context` 新增 `SessionEvent::ToolStart { input, .. }` 分支：`estimate(&input.to_string())`。
- 状态栏 ctx% 现在计入 tool-call input JSON（bash 脚本、write 文件内容、edit 等），与 `should_compact` 的 `estimate_messages`（经 `estimate_chars` 含 ToolUse）基准一致。

### D — 死配置 `prune` 移除（`config.rs` + `session_cmd.rs` + `config_contract.rs`）
- `CompactionConfig` 删除 `prune: bool` 字段、Default、parser 行。
- CLI `session show` display 移除 `prune={}`。
- 测试 JSON 移除 `"prune": true`，断言移除 `assert!(cfg.compaction.prune)`。

## 涉及文件
- `crates/session/src/prompt.rs` — 新增 `compaction_system_prompt` + `compaction_user_prompt`（+66 行）
- `crates/session/src/compaction.rs` — `summarize` 签名 + system 注入 + `compact` 提取旧 summary + `reported_tokens` 改 `input_tokens`
- `crates/tui/src/app.rs` — `track_context` +ToolStart input；修复 pre-existing `&mut cmd_tx` clippy
- `crates/core/src/config.rs` — 删 `prune` 字段/默认/解析
- `crates/cli/src/session_cmd.rs` — 删 `prune` display
- `crates/core/tests/config_contract.rs` — 删 `prune` 断言
- `crates/session/tests/prompt.rs` — 4 个新测试替换旧 `compaction_prompt` 测试
- `crates/session/tests/compaction_and_model.rs` — 新增 `reported_tokens_uses_input_only_not_total`

## 测试
| 测试 | 文件 | 覆盖点 |
|------|------|--------|
| `compaction_system_prompt_is_anchored_summarizer` | `prompt.rs` | system prompt 含 anchored + previous-summary 指令 |
| `compaction_user_prompt_has_all_structured_sections` | `prompt.rs` | 六段标题 + template 全覆盖 |
| `compaction_user_prompt_includes_previous_summary_when_provided` | `prompt.rs` | 有 prev 时含 `<previous-summary>` 块 |
| `compaction_user_prompt_without_previous_summary_says_create_new` | `prompt.rs` | 无 prev 时"Create a new" |
| `reported_tokens_uses_input_only_not_total` | `compaction_and_model.rs` | input=3k/total=12k → 不触发；input=9k → 触发 |

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 266 passed | 271 passed |
