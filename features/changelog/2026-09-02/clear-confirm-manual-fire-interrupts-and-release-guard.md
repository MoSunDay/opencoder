Commit: (working-tree, clear-confirm 手动确认立即打断执行 + Release 过滤 + 武装/确认入口对称恢复)

# clear-confirm 手动确认 running 态立即执行；kitty Release 误触与武装入口对称修复

## 背景

围绕「Shift+Tab 武装 5s 倒计时 → 再按一次立即执行」链路的用户症状：**按了没反应、卡住，很久才执行**。逐段核对 `app.rs`（事件循环）→ `clear_confirm.rs`（intercept 状态机）→ `app_loop_actions.rs`（fire）→ session 侧 apply/drain，单测层链路是通的（`shift_tab_repress_fires_armed_guard_now` 有覆盖），但真实运行时存在 3 处缺陷：

1. **running 态下「再次点击」只是排队，不是执行（主根因）**：`fire_clear_confirm` 只要 TUI `running == true`，手动确认（再按 Shift+Tab / Enter）与倒计时到期走同一条路——只把 `/act_clear_context` 原文 `handle_queue` 入队，runner 要等当前 turn 的 idle 边界（`runner/drain.rs::drain_one_queued`）才消费执行。用户感知即「卡住很久才执行」，且排队瞬间无任何 marker 反馈，视觉上等于「按键被吞」。
2. **armed 分支不过滤 `KeyEventKind::Release`**：`terminal.rs` 开启 kitty `REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES`，而 `app.rs` 的 armed intercept 分支先于 `consume_modifier_or_release`（Release 过滤所在）执行——kitty 系终端下第一次 Shift+Tab 的 **Release** 事件立刻命中 intercept 的 BackTab 臂 → 按下瞬间即 Fire，倒计时形同虚设。普通终端无 Release 事件掩盖了此问题。
3. **武装入口/确认入口不对称（d4714fd 回退引入）**：`git diff a9619d2..HEAD` 确认 d4714fd 删掉了 key_handler 的 `(Tab, SHIFT)` 分支和 BackTab 臂的 `CONTROL|ALT|SUPER` 排除守卫。后果：未武装时 `(Tab, SHIFT)` 拼写落入普通 Tab 臂 → **直接把草稿当提交/排队**；BackTab+CTRL 可武装却永远无法确认（intercept 排除 CTRL）。确认臂两拼写都在、武装臂丢了一半。

## 实现

- **A. 手动确认 running 态立即执行**（`crates/tui/src/app_loop_actions.rs`）：
  - 新增 `fire_clear_confirm_now`：`handle_confirm_key` 的 Fire 分支（手动确认）改走它——若 `running`，先复用 `cancel_running_turn` 语义打断当前 turn（cancel token + `fire_child_cancels` + `[interrupted] stopping` marker，与 double-Esc 硬中止同源），随后走既有 idle 路径 `start_turn(UiCmd::Prompt("/act_clear_context …"))` 立即起 turn。worker 侧命令 FIFO 保证时序：cancel → 当前 turn 在下一个 await/tool 边界中止 → `ResetCancel(fresh)` → Prompt 起 clear turn（`start_turn` 内部先换新 token 再发 Prompt，被取消的旧 token 不污染新 turn）。
  - **倒计时到期自动 Fire 保持入队语义不变**（无人值守的超时默认保守，绝不杀进行中的 turn），但补排队反馈：到期入队时 push `[clear] 已排队——当前轮结束后执行…` marker，消除「按键无声」感。
  - `child_runtime` / `cancelled` 两参数从 `app.rs` armed 分支调用点透传（app.rs 只改一行调用，逻辑全部收敛在 app_loop_actions.rs，不增其行数压力）。
- **B. intercept 补 kind 过滤**（`crates/tui/src/clear_confirm.rs`）：`intercept` 顶部 `k.kind == Release` 直接返回 `None`（Press/Repeat 照常），kitty 终端不再被 Release 半事件误触。
- **C. 恢复武装入口对称**（`crates/tui/src/key_handler.rs`）：恢复 `shift_tab_action` 共享函数——`(Tab, SHIFT)`（不含 Ctrl/Alt/Super）与 `BackTab`（不含 Ctrl/Alt/Super）双拼写同路：plan 模式武装 / act 模式切 plan；subagent/sidecar 聚焦时 inert（防止守卫吞掉发给聚焦窗格的 Enter）。与 `clear_confirm::intercept` 确认侧守卫逐字对称——不能确认的和弦也不能武装。
- **D. 帮助文案**（`crates/tui/src/keymap_menu/help.rs`）：Shift+Tab 条目区分「手动确认：运行中先打断当前轮再执行」与「无人确认到点自动排队」。

## 环境层吞键路径（本轮不动，仅记录）

- crossterm 0.28.1 静默丢弃 `CSI 27;2;9~`（modifyOtherKeys 格式的 Ctrl+Shift+Tab）——库层解析缺口；
- EscGuard 会吃掉 pty 分片投递的 `\x1b[Z`（Shift+Tab 转义序列被分片为 Esc 前缀 + 剩余）——传输层时序问题。
两者均不在 TUI 事件到达之后的处理链路上，属库/传输层范畴，留待后续单独处理。

## 测试覆盖（rules/01）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Release 半事件一律 inert（BackTab/Tab+SHIFT/Enter/Esc/Char 全枚举，arm 存活、composer 不被编辑） | `release_events_never_fire_or_edit` | crates/tui/src/clear_confirm.rs |
| 手动确认 running 态：打断当前轮（cancelled 标记 + interrupted marker）+ 立即发 Prompt + 不入队 | `manual_confirm_while_running_interrupts_and_fires_now` | crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs |
| 到期自动 fire running 态：仍排队 + `[clear] 已排队` marker + 无 interrupted（不打断） | `expired_guard_queues_compound_when_running`（自 `fired_guard_queues_compound_when_running` 改写） | crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs |
| Tab+SHIFT 双拼写与 BackTab 同路：plan 武装（rest/draft 载荷）/act 切 plan（草稿保留） | `tab_shift_spelling_arms_or_switches_like_backtab` | crates/tui/src/key_handler_running_mode_tests.rs |
| BackTab+CTRL/ALT/SUPER 双模式全 inert、草稿保留；Tab+SHIFT+CONTROL 被 Ctrl 守卫吞掉 | `ctrl_alt_shift_tab_chords_never_arm_or_switch` | crates/tui/src/key_handler_running_mode_tests.rs |
| subagent 聚焦时 Shift+Tab 双拼写不武装、草稿留给子会话 steer | `shift_tab_with_focused_subagent_never_arms` | crates/tui/src/key_handler_running_mode_tests.rs |

既有测试不弱化：idle fire、Esc 回撤、倒计时内键入合并、重键入取代、二次 BackTab 确认（idle）等全部保留；`handle_confirm_key` 新增 `child_runtime`/`cancelled` 参数的 6 处测试调用点同步透传。

## 回归（rules/02）

- `cargo test -p opencoder-tui`：全量 1665 passed / 0 failed。
- `cargo clippy -p opencoder-tui --all-targets` / `-p opencoder-session`：零警告。
- workspace 全量门禁见本轮提交记录。
