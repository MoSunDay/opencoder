Commit: 75d6866 / 6a5e3f3（实现与测试扩展，已提交）；working-tree post-4f4f034（评审修复：文档语义同步 + latent 复锁断言）

# $skill 激活改为 one-shot 语义：仅存活于触发它的 run，结束即清除

## 背景

skill 激活（内联 `$name` token、picker 选择、resume 恢复）原为粘性/整会话语义：激活后跨 run 持续生效，直到手动 clear 或被替换。问题：一次 `$alpha` 触发后，后续所有无关 run 都携带 `[active skill]` 尾部提醒与 latent 工具解锁，且 drain 重启会为从未提及该 skill 的 queue/steer 条目重注入 `SKILL_TRIGGER`（自续循环的放大器，见 [skill-queue-drain-semantics](../2026-08-17/skill-queue-drain-semantics.md)）。语义升级：**skill 只属于触发它的那一次 run**。

## 变更

- **`crates/session/src/skill_lifecycle.rs`（新模块，179 行）**：`clear_on_run_end`——run 结束时从内存（`skill_prompt` + `active_skill_names`）与 store（`SessionPatch{clear_skill:true}`）双侧清除，幂等守卫（无 skill → 不写 store）；`run_loop_one_shot` 包裹 run_loop，Ok/Err 两种返回都清除。
- **调用点全覆盖**：`runner/mod.rs::run`、`runner/drain.rs::run_with_registry`、`autopilot/phases.rs`（PLAN/ACT/VERIFY 三 phase 各自包裹）、`autopilot/review_pass.rs`、`autopilot/mod.rs`——每个 run_loop 入口都经 one-shot 包裹，autopilot 单 phase 作用域与既有 phase 边界重置一致。
- **崩溃保留例外**：run 中途崩溃（run-end clear 未落地）时 `sessions.skill` 保持落库，resume 恢复该 skill，续跑的 run 继续持有它直至完成——同一 run-end clear 落地。`resume.rs` 注释同步。
- **latent 工具同步复锁**：`llm_call.rs` 的 latent 解锁由 `skill_prompt_cloned()` 每 turn 派生，one-shot 清除即自动复锁（无需独立逻辑）。
- **语义注释同步（评审修复，working-tree）**：`skill_resolve.rs`、`runner/drain.rs`、`web/api.rs`（`PromptBody.skill` 的 resume 语义注释）、`tui/menu.rs`/`key_handler.rs`/`app_helpers.rs`/`worker.rs` 粘性措辞清理；记忆文档 `features/index.md`、`agents/session/index.md`、`agents/tui/index.md` 由「整会话生效/粘性」改为 one-shot 表述（`✕ clear` 行定位为手动提前/兜底清除）。

## Validation（当次实跑）

- `cargo test -p opencoder-session --test skill_one_shot` → 7 passed / 0 failed（评审修复后含新增 latent 复锁用例）
- `cargo test -p opencoder-session --test skill_queue_drain` → 4 passed / 0 failed
- `cargo test --workspace` → 见回归 gate 段
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## 测试覆盖表

| 功能 | 测试名 | 类型/文件 |
|------|--------|-----------|
| Done 后清除（内存+store） | `done_clears_skill_after_run` | integration `crates/session/tests/skill_one_shot.rs` |
| Err 后清除 | `llm_error_clears_skill_after_run` | 同上 |
| cancel 后清除 | `cancel_clears_skill_after_run` | 同上 |
| 无 skill run 为 no-op（无 store 写） | `no_skill_run_keeps_skill_none` | 同上 |
| 第二 run 无提醒、历史注入恰 1 次 | `second_run_has_no_skill_reminder` | 同上 |
| resume 中途保留、完成清除 | `resume_mid_run_keeps_skill_then_completion_clears` | 同上 |
| latent 工具随 run 结束复锁（评审新增） | `latent_tool_unlocked_then_relocked_across_runs` | 同上 |
| 清除幂等/无 skill 不写 store/无 store 会话仅清内存 ×4 | `skill_lifecycle.rs` unit `mod tests` | `crates/session/src/skill_lifecycle.rs` |
| drain 优先级回归（one-shot 下仍 pending-first） | `skill_queue_drain.rs` ×4（注释措辞随评审更新为 armed skill） | `crates/session/tests/skill_queue_drain.rs` |
| TUI 镜像同步清除 | `queued_skill_drain.rs`（run 结束后持久+内存 skill 双清断言） | `crates/tui/tests/queued_skill_drain.rs` |
| steer 替换 skill 最后者胜、run 末清除 | `steer_skill_deferral.rs` | `crates/session/tests/steer_skill_deferral.rs` |
| autopilot 单 phase 作用域 | `autopilot_skill_persist.rs` | `crates/session/tests/autopilot_skill_persist.rs` |

## Impact Surface

- 用户体验：`$skill` 提交后 skill 只作用于当次 run；状态栏 `skill:<name>` 在 run 结束后消失；`✕ clear` 变为提前/兜底清除路径（正常路径 run 结束已自动清除）。
- 行为变化：跨 run 依赖同一 skill 的旧用法（多次 plain follow-up 期望 skill 持续）需在每条 prompt 里重复 `$name` token，或经 picker 重新激活。
- 不影响：store schema（复用 `sessions.skill` 列 + `clear_skill` patch）、skill 发现/frontmatter 解析、MCP 工具过滤、压缩（尾部提醒本就不落库）。

## Related Docs

- [agents/session](../../../agents/session/index.md)（skill 生命周期段落已同步 one-shot 表述）
- [skill-queue-drain-semantics](../2026-08-17/skill-queue-drain-semantics.md)（前置：drain 优先级修复，本变更消除其「放大器」根因）
- [skill-picker](../2026-07-05/skill-picker.md)（picker 历史；`✕ clear` 行语义已更新）
