# feat(session,tui): 可排队控制命令 + plan→act handoff + 可配置 keymap + 输入/显示健壮性

## 背景

本批次整合此前散落在 working tree 的三类能力，统一为一次提交（三者存在
跨层依赖：TUI 渲染 session 新增事件、autopilot 源码与其测试跨层耦合、
terminal 新函数被 TUI 测试引用，拆分会产生不可编译的中间提交）。

1. **Session 运行时**：`/act`、`/plan`、`/act_clear_context` 升级为可排队、
   drain 感知的控制命令——切换 agent 模式不再消耗 LLM turn；
   `/act_clear_context` 通过 plan→act handoff 保留最终 plan 作为执行指令
   并重置 transcript；autopilot 的 PLAN→ACT→VERIFY 自驱动循环复用同一
   handoff 语义。三条入口（idle / queue drain / steer 边界）行为一致且能
   在 `resume()` 后正确重建。
2. **TUI keymap**：18 个全局快捷键可由用户在 `opencoder.json` 的 `keymap`
   对象中重绑（`"ctrl+h"` 等字符串规格 → `KeyCombo` 匹配器），`/short_key`
   （别名 `/sk`）弹窗支持按键捕获式实时重绑，保存后即时生效。
3. **TUI 输入/显示健壮性**：状态栏运行计时改为「最近一轮」而非会话累计；
   按住 Shift 暂停鼠标捕获以支持原生文本选择（Kitty REPORT_EVENT_TYPES
   + EnableMouseCapture 下原本无法选中文本）；若干 TUI 状态机 bugfix。

## 变更

### Session：可排队控制命令 + plan→act handoff（`crates/session/`）

- `src/control_cmd.rs`：`split_control_prefix` / `parse` 识别 `/act`、`/plan`
  （可带尾随参数 → 复合命令）、`/act_clear_context`（sentinel）；`apply` 在
  有 finalized plan 时走 `plan_handoff::handoff` 保留 plan，否则回退
  `CLEAR_CONTEXT_SENTINEL`。
- `src/runner/mod.rs`：idle 短路——裸 `/plan` 切换 agent 并 emit `Done`，
  零 LLM 调用；`handoff_pending` 标志让 ClearContext 清空 `user_text` 但保持
  `drain_mode=false` 以单 turn 执行 handoff 指令；drain-mode 下 queue 在 LLM
  调用前消费（消除 stale-agent ghost turn）；steered `/plan` 立即应用不落盘。
- `src/runner/steer.rs`：`drain_one_queued` / `claim_one_queued` 每轮弹恰好
  一条 queue，在弹出间检查 interrupt/steer。
- `src/runner/event.rs`：新增 `TranscriptReset`、`PlanHandoff`、
  `QueueConsumed{seq,text}`、`SteerConsumed{seq,text}`、`AutoPilot{phase,iteration}`
  五个 SessionEvent，含 SSE encode/decode。
- `src/resume.rs`：区分 handoff / clear-context-sentinel / compaction 三路重建，
  重新推导保留图片，handoff 存在时清零陈旧 compaction 元数据。
- `src/runner/llm_call.rs` + `src/lib.rs`：per-turn interrupt token，中断当前
  LLM turn 而不结束 `run_loop`。
- `src/bash_guard.rs`：plan 模式拦截 mutation 类 bash（重定向、写命令、git
  write、包管理器、`-c` 解释器）。
- `src/skill_resolve.rs`：inline `$name` token 剥离（compound/headless/queue）。
- `src/tools/mod.rs`：`schema_for` 按名排序，使 ChatRequest `tools` 数组确定序。

### Autopilot：PLAN→ACT→VERIFY（`crates/session/src/autopilot/`）

- `mod.rs` + `prompts.rs`：ACT 经 plan→act handoff 重置 transcript（act agent
  仅见 plan 输出作为执行指令）；无 plan 时回退注入 `execute_prompt`。VERIFY
  为隔离 shadow 一次性校验，不污染 transcript。`finish()` 在所有终止路径
  （含错误）清零 skill。

### TUI：可配置 keymap（`crates/core/` + `crates/tui/`）

- `core/src/config/keymap.rs`（新增 179 行）：`KEYMAP_INFO` 元数据表 +
  `KeymapConfig`（18 字符串字段 + Default 默认值）+ `get`/`set`。
- `core/src/config.rs` / `merge.rs` / `lib.rs`：`Config` 增 `keymap` 字段，
  merge/save 识别可编辑 keymap。
- `tui/src/keymap.rs`（314 行）+ `tui/src/keymap_tests.rs`（190 行，`#[path]`
  拆分以满足 ≤400 行）：`KeyCombo::matches`（含 raw control char / Alt 大小写
  / Tab↔BackTab 归一化）、`parse_key_spec`、`key_event_to_spec`、
  `KeyBindings::from_config`。
- `tui/src/keymap_menu/{mod,state,view}.rs`（新增）：`KeymapMenu` 状态机
  （capture / navigation 双模式、dirty 检测、`build_patch` 仅输出变更字段）+
  `render_keymap_popup` 居中弹窗。
- `tui/src/key_handler.rs` / `app_helpers.rs`：所有硬编码按键检查改为
  `bindings.<name>.matches(&k)`；新增 `switch_mode_clear`（Alt+Tab）、
  `switch_mode_keep`（Ctrl+Shift+Tab）、`force_redraw`（Ctrl+F）。
- `tui/src/app_loop.rs` / `command.rs`：`/short_key`（`/sk`）命令解析 + 弹窗
  打开；Save 时 `Config::save` + reload + 重建 `KeyBindings` 即时生效。

### TUI：输入/显示健壮性（`crates/tui/`）

- `src/terminal.rs`：`suspend_mouse_capture` / `resume_mouse_capture`
  （Shift 暂停鼠标捕获以支持原生选择）+ `consume_modifier_or_release`
  （过滤裸 modifier 按下与 key-release 事件，防 REPORT_EVENT_TYPES 双触发）。
- `src/app.rs` + `src/app_loop.rs`：`tick_clock` 增加 `prev_running`，在
  `false→true` 边界重置计时 → 状态栏显示最近一轮耗时而非会话累计。
- `src/app_loop_bugfix_tests.rs`（新增 88 行）：5 个 bug 回归——
  `OPENCODER_MODEL` env 静默回退检测、double-Esc 后 stale TurnDone 不误清
  `running`、`Done`+pending queue 激活 `drain_pending`、`TurnDone` 携带
  authoritative agent 对账丢失的 `AgentSwitch`、tick_clock 边界重置。
- `src/fmt.rs`：`format_run_duration`（`42s`/`3m5s`/`1h5m30s`）。

### TUI：tool/subagent 计时（详见同目录 `tui-tool-subagent-duration-timer.md`）

Tool/Subagent 块显示 live/frozen 运行计时，replay 路径 `elapsed_ms: Some(0)`
命中 `<1000` 守卫省略 garbage 计时。

## 测试覆盖

| 功能 | 代表测试 | 文件 |
|------|----------|------|
| 裸 `/plan` 零 LLM 调用切换 | `idle_short_circuit_switches_with_no_llm_call` | `session/tests/control_cmd.rs` |
| queue 顺序消费控制命令 | `queue_drains_control_cmds_between_real_prompts` | `session/tests/control_cmd.rs` |
| handoff 经 resume 重建 | `clear_context_survives_resume` | `session/tests/control_cmd.rs` |
| ACT 经 handoff 重置 transcript | ACT phase emits TranscriptReset | `session/tests/autopilot.rs` |
| resume 清零陈旧 compaction | resume zeroes stale summary_seq | `session/tests/handoff_clears_compaction.rs` |
| drain 模式 `` skill 消费 | `` skill consumed | `session/tests/drain_mode.rs` |
| Tab/BackTab SHIFT-optional 匹配 | `match_tab_backtab_alt_tab` | `tui/src/keymap_tests.rs` |
| `/short_key` 解析/dispatch/Enter | `parse_short_key` / `dispatch_short_key` | `tui/src/command.rs` |
| shift 暂停鼠标捕获 / modifier 过滤 | `consume_modifier_*` 系列 | `tui/src/terminal.rs` |
| per-turn 计时边界重置 | tick_clock reset 系列 | `tui/src/app_loop_bugfix_tests.rs` |
| stale TurnDone 不误清 running | `fold_stale_turndone_keeps_newer_turn_running` | `tui/src/app_loop_bugfix_tests.rs` |
| replay Tool 块省略 garbage 计时 | `replayed_tool_block_omits_duration_span` | `tui/src/session_ui/replay_duration_tests.rs` |

> I/O 纯函数（`parse_key_spec`、`format_run_duration`、`split_control_prefix`、
> `consume_modifier_or_release`）均有内联 unit 测试，零 I/O。

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 2017 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |
| 行数合规（新增 ≤400 / 迭代 ≤800） | ✅ keymap.rs 314 / keymap_tests.rs 190 / keymap_menu/state.rs 284 / app.rs 794 / render.rs 792 均 ≤800；新增文件均 ≤400 |

## Impact Surface

- **Session 契约**：新增 5 个 SessionEvent variant（`TranscriptReset` 等）——
  所有 SSE 消费方（web crate、client crate、TUI replay）已同步编解码。
- **Config 形状**：`Config` 新增 `keymap` 对象（`#[serde(default)]`，向后兼容）。
- **autopilot 源码 ↔ 测试强耦合**：`autopilot/{mod,prompts}.rs` 与
  `tests/autopilot.rs` 必须同提交（编译 + 行为双重依赖），已一并提交。
- **TUI render/handle_key/route_paste 签名变更**：所有调用方（含 8+ 测试文件）
  已同步传入新增参数。

## 备注

- 本提交合并了 working tree 中相互耦合的多个特性（session 控制命令、
  autopilot、keymap、TUI 输入/计时）。此前一次尝试拆分提交导致 test 引用
  未提交源码（`consume_modifier_or_release` import 失败）的不可编译中间态，
  故合并为单提交。
- 计时特性另见 `tui-tool-subagent-duration-timer.md`。
- 无 flaky 测试（计时断言基于 `now_ms` 确定性注入，非真实墙钟）。
