Commit: (working-tree, pre-initial-commit)

# refactor: skill token 语法从 `{$name}` 改为 `$name`

## 背景

旧的 `{$name}` 语法依赖花括号 `{}`
——这是 JSON、代码块、模板插值、正则等场景中极其常见的字符。当用户
input 同时包含 skill token 和其他含 `{}`
的内容时，`extract_skill_tokens` 可能误匹配或碎片化文本，导致 skill body
未正确注入或 clean 文本错乱。

改为 `$name` 后：`$` + 小写字母前缀的碰撞率远低于 `{$`，且无需闭合 `}`，
解析更简单、更不易出错。

## 变更

### 核心解析逻辑 — `crates/core/src/skill.rs`

重写 `extract_skill_tokens()`：

- **旧**：扫描 `{$ ... }`，花括号包裹，需要闭合 `}`，名称 trim 空格
- **新**：扫描 `$` + ASCII 小写字母 → token 开始；名称延伸 `[a-z0-9-]`；
  遇到非名称字符即终止。`$5`、`$HOME`、`$$`、尾部 `$` 均为字面文本。

测试更新：14 个既有 case 适配新语法，3 个重命名（`spaces_trimmed` →
`hyphenated_name`、`empty_name_skipped` → `dollar_then_non_alpha_is_literal`、
`unclosed_is_literal` → `name_terminates_at_non_name_char`），新增 2 个
（`dollar_uppercase_is_literal`、`double_dollar_is_literal`）。

### token 插入格式 — `crates/tui/src/key_handler.rs`

`format!("{{${}}}", name)` → `format!("${}", name)`。

### skill 展示格式 — `crates/tui/src/skill_display.rs`

`skill_token_display()`: `format!("{{${skill_name}}}")` →
`format!("${skill_name}")`；doc 注释 + 测试同步更新。

### 测试数据 + 注释（跨 21 个文件）

所有 `{$name}` 格式的测试数据、断言、注释统一替换为 `$name`：
`crates/tui/`（skill_token、app_helpers_tests、app_tests、skill_persist、
queued_skill_drain、resume_queue_display 等）、`crates/store/`
（display_text 测试 + schema/types 注释）、`crates/session/`（lib.rs、
latent.rs 注释）、`crates/cli/`（run.rs 注释）。

## 兼容性

- **不兼容旧语法**：`{$name}` 不再被识别为 skill token，会被当作字面文本保留。
- **已入库数据**：旧 session 行中 `{$name}` 格式的 display_text 仅用于 UI 展示，
  不会被重新解析，无需迁移。

## 测试清单

- `cargo build --workspace` — 通过（0 warning）
- `cargo clippy --workspace --all-targets -- -D warnings` — 通过（0 warning）
- `cargo test -p opencoder-core --lib skill` — 26 passed; 0 failed
- `cargo test -p opencoder-tui` — 全量通过（858 unit + integration）
- `cargo test -p opencoder-store --test display_text` — 5 passed; 0 failed
- `cargo test --workspace` — 全量回归通过（1760 passed; 0 failed）
