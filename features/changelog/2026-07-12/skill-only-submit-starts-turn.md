Commit: (working-tree, pre-initial-commit)

# 修复 TUI skill-only 提交不执行 skill 的 bug

## 背景

当用户在 TUI 中选中一个 skill（如 `$submit`）后，输入框只剩 `{$submit}` token。
按回车提交时：

1. `resolve_and_warn`（`app_helpers.rs`）先通过 `apply_skill_tokens` 把 skill body 写入
   `skill_handle`（即 `session.skill_prompt` 共享 `Arc<Mutex>`）
2. `clean` 被剥离 token 后为空字符串
3. 进入 `if clean.is_empty()` 分支 — 该分支**只 push `[skill: <name>]` 标记，
   不调用 `start_turn` / 不发 `UiCmd::Prompt`**

结果：skill 被设为黏滞状态并显示标记，但**不启动任何 LLM turn**，所以模型永远
读不到注入系统提示的 skill body，skill 不执行。用户必须再输入任意文本才能触发
turn — 这与 skill 的「立即执行」语义矛盾。

## 根因

`crates/tui/src/app.rs` 的 `KeyAction::Submit` 分支中，`clean.is_empty()` 路径
缺少 turn 启动逻辑。对比正常路径（`clean` 非空时）：`start_turn(... UiCmd::Prompt(clean))`
→ worker `run_session` → `run_one_llm_call` 读 `skill_prompt_cloned()` 注入
`## Active skill` 段（`prompt.rs:20-28`），skill 正常生效。

runner 层已支持空 prompt 的 drain 模式（`runner.rs:127-131`）：空 prompt 跳过
合成 user 消息但**仍调用 `run_loop` → `run_one_llm_call`**。所以用
`UiCmd::Prompt(String::new())` 即可让 skill-only 提交触发一次 turn。

doom-loop 守卫（`DOOM_THRESHOLD=3`）仅在连续 3 次相同 tool 调用时触发，不影响
单次空 prompt turn。空 prompt turn 在无 tool 调用且无队列时正常 `Done` + 退出。

## 变更

### `crates/tui/src/app.rs`（Submit 分支）

`clean.is_empty()` 分支改为：
- `active_skill.is_some()` 且 `!running`：push `[skill: <name>]` 标记后，调用
  `start_turn(&cmd_tx, &mut cancel, UiCmd::Prompt(String::new()))` 启动 drain-mode
  turn。worker 死亡则 `worker_dead` + `break`。成功则 `running = true; follow = true;
  chat.status.clear();`
- `active_skill.is_some()` 且 `running`：仅 push 标记（黏滞，等下一轮）— 行为不变
- `active_skill.is_none()`：noop — **绝不**在无 skill 时空跑 turn

### `crates/session/tests/skill_mid_run.rs`（新增测试）

`skill_only_empty_prompt_starts_turn_with_skill_in_system_prompt`：
设 skill → `run(&mut s, String::new(), ...)` → 断言 mock 恰好收到 1 次 LLM 调用
且系统提示含 skill body。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 空 prompt + skill 触发一次 turn 且系统提示含 skill body | `skill_only_empty_prompt_starts_turn_with_skill_in_system_prompt` | `crates/session/tests/skill_mid_run.rs` |
| TUI apply_skill_tokens 解析已知 skill 并写入 skill_handle | `apply_skill_tokens_resolves_and_activates_known_skill` | `crates/tui/src/app_tests.rs` |
| TUI apply_skill_tokens 报告未知 skill | `apply_skill_tokens_reports_unknown_skill` | `crates/tui/src/app_tests.rs` |
| TUI apply_skill_tokens 无 token 时不动 skill | `apply_skill_tokens_no_tokens_leaves_skill_untouched` | `crates/tui/src/app_tests.rs` |
| skill 经 steer 路径激活后出现在下一 turn 系统提示 | `skill_set_mid_run_appears_in_next_turn_system_prompt` | `crates/session/tests/skill_mid_run.rs` |
| skill 经 queue 路径激活后出现在 follow-up turn 系统提示 | `skill_set_mid_run_appears_in_queue_followup_turn` | `crates/session/tests/skill_mid_run.rs` |
| build_system 追加/省略 skill 段 | `build_system_appends_skill_when_provided` / `build_system_omits_skill_section_when_empty` | `crates/session/tests/prompt.rs` |
| start_turn worker 死亡返回 false | `start_turn_reports_false_when_worker_is_dead` | `crates/tui/src/app_tests.rs` |

- 全量回归：`cargo test --workspace` → 470 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误

## 范围之外（不在本次修）

- web 层完全无 skill 支持（`PromptBody`/`admit_and_drain`/前端均无 skill 字段）
- `resume.rs:52` 硬编码 `skill_prompt: None`，跨 resume 不持久化 skill
- 这些是独立的更大缺口，建议另开任务处理
