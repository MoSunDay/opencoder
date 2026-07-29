# feat(session/tui): 可排队的控制命令 /act, /plan, /act_clear_context

## 背景

用户需要通过斜杠命令切换运行时模式（act/plan）和/或清空上下文，且这些命令能
**排队**插入到 drain 队列中，与正常 prompt/skill 交错，在消费时立即生效，
不消耗 LLM turn。

此前模式切换只能通过 `Ctrl+Shift+Tab` 键盘快捷键，无法排队；上下文清空只能通过
`Shift+Tab`（plan→act handoff），无法从 act 模式触发。

## 设计

三个控制命令，单一真源 `control_cmd.rs`：
- `/act` → 切换到 act 模式（纯切换，不重置上下文）
- `/plan` → 切换到 plan 模式（纯切换，不重置上下文）
- `/act_clear_context` → 清空 transcript 为单条 fresh-start marker + 切换到 act

核心语义：消费时**立即生效**（非 turn boundary 的 steer），不调用 LLM。drain 循环
识别它们，应用效果，发出正确事件，继续 drain。

```
queue: [/plan] → [review skill] → [/act]
drain: switch→plan (no turn) · run "review" in plan · switch→act (no turn)
```

### ClearContext 复用 handoff 机制（无 schema 变更）

ClearContext 在 `handoff_plan` 中存储哨兵值 `<<OPENCODER_CLEAR_CONTEXT_MARKER>>`
（非 null 字节，SQLite 安全）。resume.rs 的 handoff 分支检测哨兵：匹配则重建
fresh-start marker，否则按 plan→act handoff 处理。

## 变更

### 新增

- `crates/session/src/control_cmd.rs`（280 行）— 控制命令的单一真源：
  - `ControlCmd` enum（`SwitchAgent(String)` / `ClearContext`）
  - `parse(prompt) -> Option<ControlCmd>` — 精确匹配 /act, /plan, /act_clear_context
  - `apply(session, cmd, on_event)` — 应用效果 + 持久化 + 发事件（SwitchAgent 持久化
    agent 字段修复了已知 resume 持久化缺口）
  - `fresh_start_message()` — fresh-start marker 消息构造
  - `CLEAR_CONTEXT_SENTINEL` — resume 重建用的哨兵常量
  - 6 个单元测试（parse 精确/空白/拒绝、apply switch/clear/noop）

### 修改

- `crates/session/src/lib.rs`：
  - 新增 `mod control_cmd` + re-exports（`parse_control_cmd`, `apply_control_cmd`, `ControlCmd`）
  - 提取 `store_message_count()` 方法（消除 plan_handoff 中的重复计数逻辑）

- `crates/session/src/runner/mod.rs` — 三个集成点：
  - **Idle 短路**（`run_with_registry`）：idle prompt 是控制命令时，apply + Done + return
    （跳过 run_loop + autopilot），使 CLI/Web/TUI free-text idle 可用
  - **队列拦截**（run_loop idle boundary）：重构为内层 drain 循环——控制命令 apply 后
    continue 内层（drain 下一条，无 LLM turn）；真实 prompt 记录后 break + continue 外层
  - **Steer 拦截**（turn boundary，防御性）：steered 控制命令立即 apply，不记录为 user 消息

- `crates/session/src/resume.rs`：handoff 分支检测 `CLEAR_CONTEXT_SENTINEL`，
  匹配则重建 `fresh_start_message()`，否则用 `handoff_message()`

- `crates/tui/src/command.rs`：
  - `COMMANDS` 新增 3 个条目
  - `SlashAction` 新增 `Act`, `Plan`, `ClearContext`
  - `CommandOutcome` 新增 `Queue(String)` + `#[derive(Debug)]`
  - `parse`/`dispatch` 新增对应 arm
  - 新增 `control_cmd_string(&SlashAction) -> Option<&str>` 辅助函数
  - `handle_command_key` 新增 `KeyCode::Tab` arm：控制命令 → `Queue(string)`，
    非控制命令 → `Idle`
  - 6 个新测试（parse、control_cmd_string、Tab queue/idle、Enter dispatch）

- `crates/tui/src/app_loop.rs`（`dispatch_command`）：
  - 新增 `queue_items` 参数
  - `Dispatch(Act|Plan|ClearContext)` → `start_turn(UiCmd::Prompt(cmd_str))`（无 push_user echo）
  - `Queue(string)` → running 时 admit_input(Queue) + queue_items.push；idle 时 fallback Dispatch

- `crates/tui/src/app.rs`：
  - `dispatch_command` 调用传入 `&mut queue_items`
  - Submit path：控制命令跳过 `push_user` echo（free-text polish，popup 路径无此问题）

## 测试清单（功能 → 测试名）

| 功能 | 测试 |
|------|------|
| **单元：control_cmd parse** | |
| 精确匹配 /act /plan /act_clear_context | `control_cmd::tests::parse_exact_matches` |
| 空白容错 | `control_cmd::tests::parse_trims_whitespace` |
| 拒绝非匹配 | `control_cmd::tests::parse_rejects_non_matches` |
| **单元：control_cmd apply** | |
| SwitchAgent 切换 agent + 持久化 + AgentSwitch 事件 | `control_cmd::tests::apply_switch_agent_changes_agent_and_emits` |
| ClearContext 折叠 transcript + handoff_seq + 事件 | `control_cmd::tests::apply_clear_context_collapses_and_emits` |
| 未知 agent 不变 | `control_cmd::tests::apply_switch_noop_for_unknown_agent` |
| **集成：runner** | |
| Idle 短路（0 LLM call，切换 agent） | `control_cmd::idle_short_circuit_switches_with_no_llm_call` |
| 队列 [/plan, prompt, /act] drain | `control_cmd::queue_drains_control_cmds_between_real_prompts` |
| ClearContext resume 重建 marker | `control_cmd::clear_context_survives_resume` |
| Steered /plan 不泄露为 user text | `control_cmd::steered_control_cmd_not_recorded_as_user_text` |
| **TUI：command popup** | |
| parse /act /plan /act_clear_context | `command::tests::parse_control_commands` |
| control_cmd_string 映射 | `command::tests::control_cmd_string_maps_correctly` |
| Tab 控制命令 → Queue | `command::tests::tab_on_control_command_queues` |
| Tab 非控制命令 → Idle | `command::tests::tab_on_non_control_command_is_idle` |
| Enter /act → Dispatch | `command::tests::enter_on_control_command_dispatches` |
| Enter /act_clear_context → Dispatch | `command::tests::enter_on_clear_context_dispatches` |

## 验证

- `cargo build --workspace` — 零错误（Finished `dev` profile）
- `cargo clippy --workspace --all-targets -- -D warnings` — 零警告（审查发现的 4 项已修复：await_holding_lock ×2 经块作用域释放 MutexGuard、manual_contains ×2 经 `.contains()`）
- `cargo test --workspace` — 1323 passed / 0 failed / 0 ignored（基线 1307 + 16 新增）
- 回归：`/compact`, `/config`, `/task`, `/model`; `Shift+Tab`/`Ctrl+Shift+Tab`/`Ctrl+U`;
  正常 queue/steer/drain; compaction & handoff resume — 全部不变
