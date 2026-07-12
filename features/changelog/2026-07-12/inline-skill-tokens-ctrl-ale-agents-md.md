Commit: (working-tree)

# 内联 {$name} skill token + Ctrl+A/E/L + AGENTS.md 自动注入 + app.rs 拆分

## 背景
三个独立改进合并交付：
1. **内联 skill token**：用户可在输入任意位置写 `{$skill_name}`，提交时自动解析、激活 skill 并从文本中剥离 token。取代旧的「仅空输入时 `$` 打开 picker」限制。
2. **Ctrl+A / Ctrl+E / Ctrl+L 键绑定**：光标跳首/尾 + 一键折叠所有 thinking 块/退出 subagent 视图/清空输入。
3. **AGENTS.md 自动注入**：系统 prompt 自动加载项目根/git-root/全局 `AGENTS.md` 指令文件。

## 变更

### A — 内联 skill token 系统

#### `crates/tui/src/skill_token.rs`（新文件，149 行）
- 纯函数 `extract_skill_tokens(text) -> (String, Vec<String>)`：剥离 `{$name}` token，返回干净文本 + 有序名称列表。
- 零依赖、UTF-8 安全（`{$` 检测基于 ASCII 字节，不拆多字节字符）、`{$}` 空名消耗但跳过、未闭合 `{$abc` 当字面 `{` 处理。
- 15 个单元测试（无 token / 孤 `$` / 文中 / 多个 / 相邻 / 带空格 / 空 / 未闭合变体 / 双花括号 / UTF-8 / dash 名 / 空输入）。

#### `crates/tui/src/app_helpers.rs`
- `apply_skill_tokens`：解析 token → 去重保序 → `discover_skills()` 匹配 → 写入 `active_skill`/`active_skill_body`/`sys_tokens` + 写入共享 `skill_handle: Arc<Mutex<Option<String>>>`（= `session.skill_prompt`）。返回 `(clean_text, unresolved_names)`。
- `resolve_and_warn`：`apply_skill_tokens` 包装 + 未解析名推黄色 `⚠ unknown skill` marker。
- **skill-only 提交**（剥离后 clean text 为空但 skill 已激活）：显示 `[skill: …]` marker，不启动 LLM turn。

#### `crates/tui/src/key_handler.rs`
- skill picker 选中时插入 `{$name}` token 到光标（`composer::insert_str`），返回 `KeyAction::None`（body 在提交时解析）。
- `$` 触发条件从 `input.is_empty() && cursor_idx == 0` 放宽为任意位置。

#### `crates/tui/src/app.rs`
- Submit/Steer/Queue 三个 `KeyAction` 分支统一调 `resolve_and_warn`。

### B — Ctrl+A / Ctrl+E / Ctrl+L 键绑定

#### `crates/tui/src/key_handler.rs`
- CONTROL 块新增 `Char('a')` → `cursor_idx = 0`、`Char('e')` → `cursor_idx = input.chars().count()`（char 安全，与 Home/End 一致）。

#### `crates/tui/src/chat.rs`
- `ChatView::collapse_all_thinking()`：遍历所有 Thinking 块设 `collapsed = true`。

#### `crates/tui/src/app_helpers.rs`
- `pre_key_intercept`：提取 Esc-subagent 退出 + Ctrl+L 拦截（折叠所有 thinking / 退出 subagent / 清空输入）为独立函数，返回 `bool`（true = 已消费，调用方 continue）。

#### `crates/tui/src/keybind.rs`
- 新增 Ctrl+A/E、Ctrl+L help 行；更新 `$` 描述。

### C — AGENTS.md 自动注入

#### `crates/session/src/prompt.rs`
- `build_system` 前置 `## Project instructions` 块，从 3 级优先级加载：`~/.opencode/AGENTS.md` → git-root `AGENTS.md`（从 working_dir 向上找 `.git`）→ `working_dir/AGENTS.md`。大小写不敏感、canonicalize 去重、不可读静默跳过。
- 新增 `load_instructions`/`find_agents_md`/`find_git_root` 私有助手。

#### `crates/session/Cargo.toml`
- 新增 `dirs` 依赖。

### D — app.rs 行数控制（≤800）

#### `crates/tui/src/app_helpers.rs`
- 新增 `pre_key_intercept`（Esc + Ctrl+L 拦截）+ `handle_mouse`（鼠标事件全处理）+ `resolve_and_warn`，从 app.rs 提取。
- app.rs 从 896 行降至 740 行。

### E — 其他
- `crates/core/src/agent.rs`：PLAN_SUFFIX 改写为更紧凑的指令 + 强制 Goal/TODO/Verify/Risks/Align 段落。
- `crates/tui/src/app.rs`：session resume 时 re-sync sticky skill（`skill_handle` 回写到新 worker）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| token 剥离 + 名称提取（多场景） | `extract_skill_tokens_*`（15 个） | `skill_token.rs` |
| 已知 skill 解析 + 激活 + skill_handle 写入 | `apply_skill_tokens_resolves_and_activates_known_skill` | `app_tests.rs` |
| 未知 skill 报告 + 不修改状态 | `apply_skill_tokens_reports_unknown_skill` | `app_tests.rs` |
| 无 token 时 sticky skill 保留 | `apply_skill_tokens_no_tokens_leaves_skill_untouched` | `app_tests.rs` |
| Ctrl+A 光标到首 | `ctrl_a_moves_cursor_to_start` | `app_tests.rs` |
| Ctrl+E 光标到尾 | `ctrl_e_moves_cursor_to_end` | `app_tests.rs` |
| Ctrl+A/E 空输入 | `ctrl_a_e_on_empty_input_stay_at_zero` | `app_tests.rs` |
| Ctrl+A/E 多字节 char 安全 | `ctrl_a_e_handle_multibyte_chars` | `app_tests.rs` |
| 折叠所有 thinking 块 | `collapse_all_thinking_collapses_every_block` | `chat_tests.rs` |
| 无 thinking 块时 noop | `collapse_all_thinking_noop_without_thinking_blocks` | `chat_tests.rs` |
| AGENTS.md 加载（8 场景） | `agents_md_*` / `instructions_*` | `session/tests/prompt.rs` |
| skill 运行中设置下轮生效 | `skill_set_mid_run_appears_in_next_turn_system_prompt` | `session/tests/skill_mid_run.rs` |
| skill 队列后续轮生效 | `skill_set_mid_run_appears_in_queue_followup_turn` | `session/tests/skill_mid_run.rs` |

- 全量回归：`cargo test --workspace` → 469 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
