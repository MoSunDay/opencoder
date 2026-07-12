# Plan-mode subagent guard 硬化：测试补全 + skill_mid_run 修复

## 背景
plan-mode subagent guard（`runner.rs:503`）在 `resolve_agent` 之前拦截 plan agent 的 `build` subagent 请求，但缺乏充分测试保护：
1. `base_prompt_plan()` 的 `.replace()` 无测试保护——改 BASE_PROMPT 措辞会静默泄漏 build 广告
2. 无 act→build 正向测试——guard 条件写错也不会被发现
3. 测试 doc comment 与断言矛盾
4. 「不产生写操作」合约未在文件系统层面验证

此外 `skill_mid_run.rs` 因 Rust 类型推断问题（`Arc<dyn ChatStream>` 向后传播）无法编译，阻塞 `cargo test --workspace`。

## 变更

### A — `.replace()` 保护测试（`crates/core/src/agent.rs`）
- 新增 `#[cfg(test)] mod tests`，测试 `plan_prompt_strips_build_subagent_advertisement`：
  - 断言 `base_prompt_act()` 包含 `.replace()` 目标子串（检测 BASE_PROMPT 措辞漂移）
  - 断言 `base_prompt_plan()` 不含 `'build' (full tools)`（安全属性）
  - 断言 `base_prompt_plan()` 仍含 `'explore' (read-only)`（replace 不过度剥离）

### B — act→build 正向测试（`crates/session/tests/plan_mode_subagent_guard.rs`）
- 新增 `act_mode_allows_build_subagent`：act agent 发 `subagent_type="build"` → 断言 `SubagentStart{kind:"build"}` + `SubagentEnd{ok:true}`
- 防止 guard 条件误伤合法路径（如漏 `AgentKind::Plan` 条件）

### C — doc comment 修复（`crates/session/tests/plan_mode_subagent_guard.rs`）
- 将 "must NOT mention 'build'" 改为 "must NOT advertise 'build' as a valid option"
- 明确区分「回显被拒类型名」（允许）与「广告为有效选项」（禁止）

### D — 文件系统写操作验证（`crates/session/tests/plan_mode_subagent_guard.rs`）
- `plan_mode_blocks_build_subagent` 末尾新增 `std::fs::read_dir` 断言：tempdir 必须仍为空

### E — skill_mid_run 类型推断修复（`crates/session/tests/skill_mid_run.rs`）
- 两个测试函数的 `let mock = Arc::new(...)` 加 `: Arc<MockChatClient>` 类型标注
- 原因：`let client: Arc<dyn ChatStream> = mock.clone()` 导致 Rust 双向类型推断将 `mock` 也推断为 `Arc<dyn ChatStream>`，使 `mock.requests()` 找不到方法

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| .replace() 不变式保护 | `plan_prompt_strips_build_subagent_advertisement` | `core/src/agent.rs` |
| plan 模式阻止 build subagent | `plan_mode_blocks_build_subagent` | `session/tests/plan_mode_subagent_guard.rs` |
| plan 模式允许 explore subagent | `plan_mode_allows_explore_subagent` | `session/tests/plan_mode_subagent_guard.rs` |
| act 模式允许 build subagent | `act_mode_allows_build_subagent` | `session/tests/plan_mode_subagent_guard.rs` |
| skill 运行中设置下轮生效 | `skill_set_mid_run_appears_in_next_turn_system_prompt` | `session/tests/skill_mid_run.rs` |
| skill 队列后续轮生效 | `skill_set_mid_run_appears_in_queue_followup_turn` | `session/tests/skill_mid_run.rs` |
| skill set/clone 往返 | `set_skill_and_clone_roundtrip` | `session/tests/skill_mid_run.rs` |
| with_skill builder | `with_skill_builder_sets_skill` | `session/tests/skill_mid_run.rs` |

- 全量回归：`cargo test --workspace` → 470 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- e2e 语法：`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py` → exit 0
