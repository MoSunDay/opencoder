Commit: (working-tree, post-ac250e5)

# sticky skill 可清除性修复：autopilot 自产自清 + `$` picker 恢复 clear 行

## Context

skill 的 sticky（整会话黏贴直至换选/clear）是设计契约，符合预期；但本轮排查确认两个真缺陷：

1. **系统注入的 review skill 关不掉**：autopilot（`drive` / `review_pass`）激活的 review skill 是系统塞的，`review_pass` 的 `set_skill(None)` 在 `run_loop(...)?` 之后——LLM 出错（如 429 耗尽）即跳过清理；且清理只改内存不落库，resume 从 `sessions.skill` 列复活。用户从未选择该 skill，却既躲不开又关不掉。
2. **TUI 无轻量清除手段**：`$` picker 的「✕ clear」行早已被删，`KeyAction::SetSkill(None)` 成死代码；且即便复活，`apply_skill_selection(None)` 落库写的是 `SessionPatch { skill: None }`——store 语义里 `skill: None` = 不动，属「假持久化」，resume 仍复活。

次要不一致一并修复：`SwitchAndStart` 无 plan 的 fallback 只清内存不落库；subagent child / `replay_child` 继承父 config 的 `autopilot.mode`，子会话会在 scoped task 后进入 `drive`/`review_pass` 自驱循环。

## Change Summary

- **autopilot 自产自清（Ok/Err 双路径 + 落库）**（`crates/session/src/autopilot/`）：
  - 新增 `clear_injected_skill(session)`（mod.rs）：`set_skill(None)` + best-effort `SessionPatch { clear_skill: true }` 落库——系统注入的 skill 绝不外泄到 resume。
  - `review_pass`：`run_loop` 结果先绑定再清理——**错误路径同样清 skill + 发 `Done`** 后传播错误（对齐 `drive` 错误路径的终态簿记）。
  - `drive::finish` 改 async 并走 `clear_injected_skill`（Complete/MaxIterations/Cancelled/Aborted 全路径落库）。
  - `phases.rs::run_act_phase` 无 plan fallback：内存清理升级为 `clear_injected_skill`（防 crash/resume 复活）；handoff 分支原有合并 patch 不变。
- **恢复 TUI 用户清除手段**（`crates/tui/src/`）：
  - `menu.rs`：未过滤列表**尾部**恢复「✕ clear — deactivate the sticky skill」行（`Row::Clear`；键入过滤时隐藏，防止误触）；`Enter/Tab` 在该行返回 `MenuOutcome::Clear`；两个渲染变体（居中浮层/下拉）同步；零 skill 时 clear 行仍可达（sticky skill 可比其 SKILL.md 文件活得久）。
  - `key_handler.rs`：`MenuOutcome::Clear → KeyAction::SetSkill(None)`，摘掉 `#[allow(dead_code)]` 复活该路径。
  - `skill_persist.rs::apply_skill_selection`：`None` 分支改写 `SessionPatch { clear_skill: true }`（修掉 `skill: None` 假持久化）；`Some` 分支行为不变。
- **子会话强制 `ApMode::Off`**：`runner/subagent.rs`（child 创建后）与 `resume.rs::replay_child`（resume 后）显式置 Off——autopilot 是顶层编排，子会话只跑 scoped task。
- **`SwitchAndStart` 无 plan fallback 落库**（`worker.rs`）：else 分支补 `clear_skill: true` 持久化，与 handoff 分支对齐。
- **P2**：`core/config/autopilot.rs::merge` 对存在但不可解析的 `mode` 打 `tracing::warn`（行为不变：保留宽松 fallback 链，legacy `enabled` 仍生效）。
- sticky 语义本身**原样保留**：无 `$token` 普通提交不清 skill（`skill_resolve.rs` 不动）。

## Validation（当次实跑）

- `cargo test --workspace --no-fail-fast`：175 套件全绿，**2855 passed / 0 failed**。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo build --workspace`：编译干净。
- 备注：首轮全量跑中 `tools::bash::tests::bash_normal_completion` 在高并发下闪挂一次（隔离重跑两次均绿，与本轮改动无关的既有 flake）；工作树含并行会话的在途改动，总数含其贡献。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `session/tests/autopilot_review.rs::review_mode_error_still_clears_and_persists_skill` | integration | review turn LLM 出错：内存清 + `Done` 照发 + store `skill` 列 NULL（防 resume 复活） |
| `session/tests/autopilot.rs::drive_clears_persisted_skill_on_complete_and_on_error` | integration | `drive` Complete 与 phase-error 双场景：store `skill` 列均 NULL |
| `session/tests/subagent.rs::subagent_child_never_enters_autopilot` | integration | 父 config `ap` 模式下子会话 SubagentStart..SubagentEnd 窗口内零 `AutoPilot` 事件、父自身照常自驱 |
| `session/tests/resume_replay.rs::replayed_child_never_enters_autopilot` | integration | `replay_child` 强制 Off：恰好一个 child turn、task Completed(ok)，无 drive 消耗 |
| `tui menu.rs::clear_row_shown_only_on_unfiltered_list` | unit | clear 行只在未过滤列表出现，键入即隐藏 |
| `tui menu.rs::enter_on_clear_row_returns_clear_outcome` / `tab_on_empty_menu_confirms_clear_row` | unit | clear 行确认键 → `MenuOutcome::Clear`；零 skill 时仍可选 |
| `tui skill_persist.rs::apply_skill_selection_none_persists_clear` / `..._some_persists_body` | unit | clear 走 `clear_skill:true` 真落库；set 路径不回归 |
| `tui/tests/agent_switch_persist.rs::switch_and_start_without_plan_persists_skill_clear` | e2e | `SwitchAndStart` 无 plan fallback：内存清 + store 清对齐 |
| `core config/autopilot.rs::unknown_mode_warns_but_keeps_legacy_fallback` | unit | 坏 mode 警告后 legacy `enabled` fallback 语义保持不变 |

另修 3 个受 clear 行影响的既有 menu 单测断言（visible_count 计入尾部 clear 行、wrap 路径、filter 后无 Clear 行）。无删测试 / 无 `#[ignore]` / 无弱断言。
