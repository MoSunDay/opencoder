Commit: (working-tree, pre-initial-commit)

# fix(tui): SwitchAgent 切换到 plan 时重置 plan_input_count（对齐 control_cmd）

## 背景

TUI 内经按键（Alt+Tab / Ctrl+T）切换 agent 模式走 `UiCmd::SwitchAgent`，而
`/plan` 这类 slash-command 走 `control_cmd::apply`。两条路径在同一份
`SessionState` 上产生了行为分歧：

- `control_cmd::apply` 的 `SwitchAgent` 分支在切到 `plan` 时把
  `session.plan_input_count` 清零（`crates/session/src/control_cmd.rs`）。
- `worker.rs::process_cmd` 的 `SwitchAgent` 分支只换 `sess.agent` + 广播事件 +
  落库，**从不重置** `plan_input_count`。

后果：先在 plan 模式跑过几轮（计数 > 0），再用 TUI 按键切回 plan 时，计数仍是旧
值，`maybe_tag_plan_prompt`（`crates/session/src/lib.rs`）会错误地提前追加
"当前处于只读的 plan 模式" 提醒，或导致 plan 阶段的提醒计数错位。Web 侧无此问题
（`resume` 每次从 0 重建会话，`resume.rs`），故该分歧只影响 TUI。

> 备注：`DOOM_THRESHOLD` 文档值（`agents.md`/`README.md`/`README.en.md`）经核对
> 已与代码常量 `crates/session/src/runner/event.rs`（`=20`）一致，无需改动。

## 变更

### TUI `SwitchAgent` 对齐 control_cmd（`crates/tui/src/worker.rs`）

- **`crates/tui/src/worker.rs`**：`UiCmd::SwitchAgent(name)` 分支在
  `sess.agent = a;` 之后新增
  `if name == "plan" { sess.plan_input_count = 0; }`，与
  `control_cmd::apply` 完全一致。仅 `name == "plan"` 时触发，其余 agent 不受
  影响；现有的事件转发 / `persist_session_agent` 落库逻辑不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 切到 plan 重置 plan_input_count（TUI 路径对齐 control_cmd） | `switch_agent_to_plan_resets_plan_input_count` | crates/tui/tests/agent_switch_persist.rs |
| SwitchAgent 落库并跨 resume 保持（既有，回归保护） | `switch_agent_persists_mode_and_survives_resume` | crates/tui/tests/agent_switch_persist.rs |
| plan→act SwitchAndStart 落库 act（既有，回归保护） | `switch_and_start_handoff_persists_act_mode` | crates/tui/tests/agent_switch_persist.rs |

- 全量回归：`cargo test --workspace` → **1685 passed / 0 failed / 1 ignored**
  （当次实跑；1 ignored = `tools::research::tests::research_smoke_bing_wikipedia`，
  需真实 Chrome + 网络，预先存在，与本次改动无关）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净编译
- 行数：worker.rs 412（≤800）、agent_switch_persist.rs 220（≤800）
- 防修绿：本次 diff 纯加法（worker.rs +8、测试 +41），0 删除测试、0 新增
  `#[ignore]`，新断言均为可观测状态（`assert!(!quit)`、
  `assert_eq!(sess.agent.name, "plan", ..)`、`assert_eq!(sess.plan_input_count, 0, ..)`）

## Impact Surface

- TUI 经按键切到 plan 时，`plan_input_count` 正确归零，plan 阶段提醒逻辑从新阶段
  开始计数。
- 不影响：CLI（走 `control_cmd` 路径，本就重置）/ Web（每次 resume 重建，计数从
  0 起）/ store schema / prompt 契约 / 跨进程 resume（`plan_input_count` 为运行时
  内存状态，不持久化）。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [agents/session](../../../agents/session/index.md)
- [既有相关 changelog：SwitchAgent 落库](../2026-08-01/tui-agent-mode-persist.md)
