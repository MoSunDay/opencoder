# fix(skill): 未解析 `$token` 不再吞吃用户输入内容

## 背景

`extract_skill_tokens`（`crates/core/src/skill.rs`）会从输入中剥离 **所有**
`$name` token —— 无论该名称是否对应真实 skill。下游解析器（TUI
`apply_skill_tokens_with` 与 runner `resolve_inline_skills_with`）随后将名称划分为
resolved / unresolved，但返回的 `clean` 文本中 **unresolved token 的字节已被剥离且从未恢复**。

结合 skill-name 字符集 `[a-z0-9-]` 的贪婪匹配，造成可复现的数据丢失：

- TUI `$`-picker 在光标处插入裸 `$review`（**无尾随空格**）。
- 用户直接键入数字 → 缓冲区变为 `$review1) 修登录bug\n2) 加测试`。
- 贪婪扫描将 `review1` 当作 skill 名称 → `extract_skill_tokens` 剥离整个 `$review1`
  → clean = `") 修登录bug\n2) 加测试"`，**`1` 永久消失**。
- 模型收到残缺文本 → 推理错误。

该 bug 影响全部三条交付路径（TUI/Enter、CLI/run、web/queue-steer），因为它们共享
同一个核心剥离函数。

## 变更

### 1. 核心修复 — `crates/core/src/skill.rs`

新增纯函数 `strip_resolved_skill_tokens(text, resolved: &HashSet<String>)`：重扫
`text`，只跳过 `$name` ∈ resolved 的 token；unresolved 的 `$name` 原样保留为字面量。
UTF-8 安全（字节级扫描，token 字节均为 ASCII）。`extract_skill_tokens` 保持不变
（仍用于发现/激活 + warn）。

### 2. TUI 解析器 — `crates/tui/src/app_helpers.rs`

`apply_skill_tokens_with` resolve 后，用 resolved 名集合调用
`strip_resolved_skill_tokens` 重建 `clean`，替换直接复用 `extract_skill_tokens`
的 clean。unresolved 文本现在完整保留。

### 3. Runner 解析器 — `crates/session/src/skill_resolve.rs`

`resolve_inline_skills_with` 同样改用 `strip_resolved_skill_tokens` 重建 clean，
与 TUI 保持一致。

### 4. Picker 分隔符 — `crates/tui/src/key_handler.rs`

`format!("${}", name)` → `format!("${} ", name)`（尾随空格）。保证 `$review` 后
必有分隔符，后续输入不会粘到 token 名称上导致假名吞吃。Shift+Enter 多行 trim
后结果不变。

## 测试清单

| 路径 | 测试 | 文件 |
|------|------|------|
| 核心 | `strip_resolved_greedy_glued_name_preserved_verbatim` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_space_separated_resolved_drops_token` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_keeps_unresolved_verbatim` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_mixed_tokens` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_empty_input` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_literal_dollar_untouched` | `crates/core/src/skill.rs` |
| 核心 | `strip_resolved_utf8_preserved` | `crates/core/src/skill.rs` |
| session | `unresolved_skill_reported_and_skill_untouched`（更新断言） | `crates/session/src/skill_resolve.rs` |
| session | `mixed_resolved_and_unresolved`（更新断言） | `crates/session/src/skill_resolve.rs` |
| TUI | `apply_skill_tokens_reports_unknown_skill`（更新断言） | `crates/tui/src/app_helpers_tests/skill_apply.rs` |
| TUI | `apply_skill_tokens_combined_mixed_resolved_and_unresolved`（更新断言） | `crates/tui/src/app_helpers_tests/skill_apply.rs` |
| TUI | `skill_menu_enter_picks_selected_skill`（更新断言） | `crates/tui/src/app_tests/skill_tests.rs` |
| TUI | `pick_inserts_token_at_cursor_mid_text`（更新断言） | `crates/tui/src/app_tests/skill_tests.rs` |
| 集成 | `glued_skill_token_preserves_numbered_list` | `crates/tui/tests/skill_glue_content_preserved.rs` |

**当次实跑**: `cargo test --workspace` → 1839 passed; 0 failed; 0 ignored。
`cargo clippy --workspace --all-targets -- -D warnings` → 0 warning。
