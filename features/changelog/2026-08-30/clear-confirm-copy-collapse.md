Commit: (working-tree, Shift+Tab 倒计时确认防护 arm 后提示收敛为单条倒计时文案)

# Shift+Tab 倒计时 chip 文案收敛：单条「{N}s 之后仅保留计划并执行…」（含「有意保留」清单）

## 背景

38cbd84（2026-08-29，[/act_clear_context canonical 更名 + Shift+Tab 倒计时确认防护](../2026-08-29/clear-context-countdown-guard.md)）落地了 arm→fire/cancel 的倒计时确认防护，但 arm 后的提示偏多：chip 写成 "→ clear {N}s 后清空上下文"（`frame::is_warn_flash` 靠 `→ clear` 前缀判 warn），`engage()` 往 transcript 打了 3 条 `[clear]` 标记（含 seed 预览回显「┃」行、「当前无可保留的回复」变体、Esc/Enter 提示行），信息重复且文案把「清空」说得太重——实际语义是「保留最后回复作接续种子，仅保留计划并执行」。本轮把 arm 后提示收敛为仅一条「{N}s 之后仅保留计划并执行…」，chip 增加 braille spinner 随 anim_tick 转动形成倒计时动画；**零行为变更**，状态机、时间窗口与按键语义全部未动。

## 实现

- **crates/tui/src/clear_confirm.rs**（325 行）：
  - `banner(cc, now, anim_tick)` chip 文案 → `"→ {spin} {N}s 之后仅保留计划并执行…"`；`spin` 复用状态栏 10 帧 braille `SPINNER`（render.rs 一行 `pub(crate) use status_bar::SPINNER;` 再导出），随 anim_tick 转动 = 倒计时动画。
  - `engage()` 原 3 条 `[clear]` transcript 标记收敛为 1 条 `[clear] 5s 之后仅保留计划并执行…`；删除 seed 预览回显「┃」行、「当前无可保留的回复」变体、Esc/Enter 提示行。
  - `preview_text` 因唯一调用方删除而成死代码，连函数带其专属测试一并移除（crates/tui 内零残留）。
- **crates/tui/src/frame.rs**（152 行）：`is_warn_flash` 第三臂由 `starts_with("→ clear")` 改为 `contains` 匹配倒计时文案「之后仅保留计划并执行」（warn 橙语义不变），doc 同步为 "matched by its countdown banner text, not by a prefix"。
- **crates/tui/src/frame/tests.rs**：新增倒计时文案正例（含 braille spin 字符的完整 chip 形态）+ 旧 `→ clear 5s 后清空上下文` 负例（"pre-anim banner wording must no longer match"）；`→ act mode`/`busy` 负例保留。
- **crates/tui/src/keymap_menu/help.rs**:18：Shift+Tab 条目按键说明改为「先倒计时确认，Esc 回撤 / Enter 提前执行」——chip 不再展示按键可供性，帮助页兜底。
- **crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs**（230 行）：断言更新为新文案 + chip 内容 + `markers.len() == 1`；arm/fire/queue/Esc 回撤语义断言未动。

## 行为不变项（本轮零触碰）

以下全部未动，本轮仅渲染层文案与动画，后续迭代勿顺着文案改动误伤语义：

1. arm→fire/cancel 状态机。
2. 5s 倒计时窗口（`CLEAR_CONFIRM_WINDOW_MS`）。
3. Esc 回撤（恢复草稿、同帧清掉倒计时 chip）。
4. Enter 提前执行。
5. 其余按键 inert（不打断倒计时）。
6. fire 时保留最后一条回复作接续种子的路径。

## 有意保留 / 勿误伤

1. **ap_menu 的 "Esc 取消"**：autopilot 菜单 footer 的 " [y]/Enter 全局    [n] 仅本会话    Esc 取消 "（`crates/tui/src/ap_menu/view.rs`），是另一功能的按键可供性文案，与 clear-confirm 无关，勿并入本轮「收敛按键提示」的范围。
2. **英文注释/断言里的 "→ clear"**：`key_handler_plan_edit_tests.rs` 的 `// Down goes past the end → clears`（箭头是流程示意）与 `worker/tests.rs` 的 `// … → clear is allowed`（"允许清空" 的英文表述）——均非 banner 文案，按 `→ clear` 字面 grep 清理时会误报，勿动。
3. **`is_warn_flash` 的 "→ sandbox mode" / "→ edit plan" 两臂**：既有语义（上一轮 [plan 卡片编辑器闪烁文案纠偏](../2026-08-29/plan-edit-flash-reword-legacy-guard-notes.md) 落定），本轮只动第三臂。
4. **cancel 路径标记**：`[clear] 已取消（回撤）— 上下文未清空`（clear_confirm.rs cancel 分支）仍保留——属回撤语义标记，不在「arm 后提示收敛」范围内。

## 验证记录

- 旧文案 grep（`后清空上下文|即将清空上下文|接续种子|当前无可保留|Esc 取消（回撤）`）crates/tui → 0 命中（零残留，当次实跑）。
- `cargo test -p opencoder-tui`：26 套件 / 1563 passed / 0 failed（2026-08-30 01:29 一手实跑）。
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings`：零告警（一手实跑）。
- 全量 `cargo test --workspace`：被并行在途改动阻断——crates/session/tests/control_cmd.rs:13 unclosed delimiter。该文件属另一在途任务，非本轮触达面（本轮 diff 仅 crates/tui）；workspace 级复跑待其收敛后再补。
- 行数合规（迭代 ≤800，本轮零新增源码文件）：clear_confirm.rs 325 / frame.rs 152 / render.rs 789 / help.rs 228 / act_clear.rs 230。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| arm 后 chip 收敛为单条倒计时文案（braille spinner 动画 + engage 单条 `markers.len()==1` 标记） | `slash_clear_context_arms_countdown_guard` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |
| arming/banner/engage 文案单元路径（chip 含「之后仅保留计划并执行」、标记含「5s 之后仅保留计划并执行」） | `maybe_arm_arms_only_clear_context_text` | `crates/tui/src/clear_confirm.rs` |
| `is_warn_flash` 第三臂 contains 匹配新倒计时文案，旧「→ clear …后清空上下文」负断言不再命中 | `warn_flash_hue_covers_sandbox_plan_and_clear_guard` | `crates/tui/src/frame/tests.rs` |
| Esc 回撤清除倒计时 chip（升起→消失断言） | `esc_cancel_drops_countdown_chip` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |

- `preview_text` 随唯一调用方删除而成死代码并一并移除，其专属测试 `preview_text_squashes_and_truncates` 随之消亡（随死代码删除，非为修绿删测试；本轮唯一删除项）。
- 全量回归：`cargo test --workspace` → 233 套件 / 3317 passed / 5 failed（tip `0108e5c` 内容树一手实跑，`/tmp/oc_r2_ws_test.log`）：smoke 1 项与 runner_control 3 项低负载 solo 复跑全 PASS（负载饥饿）；runner_cancel target 1 项（`unknown_session_reports_ok_false`）在 base `64a4878` 同样 FAILED → pre-existing flake，低负载窗口复跑待补。
- scoped 回归：`cargo test -p opencoder-tui` → 26 套件 / 1556 passed / 0 failed（`/tmp/oc_r2_tui.log`，TUI_EXIT=0）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（CLIPPY_EXIT=0，隔离干净树实跑）。

## 关联

- 前因：38cbd84，[/act_clear_context canonical 更名 + Shift+Tab 倒计时确认防护](../2026-08-29/clear-context-countdown-guard.md)（倒计时确认防护引入处，本轮收敛其 arm 后提示）。
- 相关：[plan 卡片编辑器闪烁文案纠偏 + switch_mode 删除后的误导文案清理](../2026-08-29/plan-edit-flash-reword-legacy-guard-notes.md)（`frame::is_warn_flash` warn 臂上一轮清理；本轮第三臂由前缀匹配改 contains）。
