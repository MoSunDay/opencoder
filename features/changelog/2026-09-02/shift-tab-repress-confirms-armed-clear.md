Commit: 958b5fa (plan 倒计时 armed 态再按 Shift+Tab 立即提交)

# armed 倒计时再按一次 Shift+Tab 立即提交（与 Enter 同效，确认 chord 复用）

## 背景

plan 模式 Shift+Tab（或文本 `/act_clear_context`）arm 5s 倒计时后，用户报
「再提交一次**或**再按一次 Shift+Tab 就立刻提交」——前半句（Enter 立即提交）由
[clear-confirm 倒计时内提交立即执行](../2026-09-01/clear-confirm-submit-fires-now.md)
（71e7e1e）解决，后半句是缺口：拦截层只有 `KeyCode::Enter` → Fire 分支，arming
形状 `BackTab` 落 `_ => None` 兜底被吞，再按 Shift+Tab 无效，只能等满 5s 或改按
Enter。本条目补齐：确认 chord 与 arming chord 同形——armed 态再按一次 Shift+Tab
与 Enter 完全同效，立即提交。

## 实现

- **`crates/tui/src/clear_confirm.rs::intercept`** 补两条 Fire 分支（根因即原
  `_ => None` 兜底吞键）：
  - `KeyCode::BackTab` 且 modifiers 不含 CONTROL|ALT|SUPER → Fire（部分终端对
    BackTab 剥掉 SHIFT 旗标，故按裸形状匹配）；
  - `KeyCode::Tab` + SHIFT 且不含 CONTROL|ALT|SUPER → Fire（同一和弦的另一终端
    形状）。
  - **有意收窄**：Ctrl/Alt/Super 和弦永不触发确认——已退役的 ctrl+shift+tab 以
    `BackTab+CONTROL|SHIFT` 形状落在这里（pane 切换类），放行会把一个曾经的
    无害模式切换和弦变成即时破坏性 clear。模块 doc 同步。
- **fire 复用 Enter 既有分支**（`app_loop_actions.rs::handle_confirm_key`）：
  倒计时内键入文本先经 `merge_typed` 并入复合尾部、running 原文排队——附加需求
  合并与排队语义零漂移。
- **`keymap_menu/help.rs`**：键位文案改为「提交（Enter）或再按一次 Shift+Tab
  立即执行」；`app.rs` armed 分发注释同步。
- **优雅降级**：异常终端若发出既非裸 BackTab 又非 Tab+SHIFT 的形状，落
  `_ => None` inert——倒计时照常到点自动触发，Enter 仍即时可用，无数据面风险。

## 测试清单（功能点 → 测试）

| 功能点 | 测试 |
|---|---|
| armed 态再按 Shift+Tab 立即提交（dispatch 全链路：arm 被消费 + canonical prompt 断言） | `app_loop_dispatch_cmd_tests/act_clear.rs::shift_tab_repress_fires_armed_guard_now` |
| 拦截层 Shift+Tab 再按与 Enter 提交同效（BackTab 裸 / Tab+SHIFT 双形状） | `clear_confirm.rs::intercept_shift_tab_repress_fires_like_submit` |
| Ctrl/Alt/Super 和弦保持 inert（退役 ctrl+shift+tab 不误触） | `clear_confirm.rs::intercept_ctrl_alt_shift_tab_chords_stay_inert` |
| 既有语义保留：Enter fire、armed 可编辑、running 排队合并、Esc 回撤 | `clear_confirm.rs::intercept_enter_fires_and_leaves_arm_for_caller`、`act_clear.rs::fired_guard_queues_compound_when_running`、`act_clear.rs::esc_cancel_drops_countdown_chip` 等全部保留 |

门禁读数（**提交树快照本体**：`git checkout-index` 导出 958b5fa 独立跑，读数即
提交自身）：

- `cargo test -p opencoder-tui` → **1639 passed / 0 failed**（27 套件）
- `cargo clippy --all-targets -- -D warnings` → 零告警
- 零漂移：`git diff 958b5fa HEAD -- crates/tui` 为空（后续提交未触碰 tui），
  快照读数即描述当前 HEAD
- 真机形式确认（plan 模式倒计时中再按 Shift+Tab → transcript 出现
  `/act_clear_context …` 并开跑）待用户下次进入 TUI 时顺手验证；终端形状同一性
  已由代码路径闭合：arming 成功 ⇒ 终端发出裸 BackTab ⇒ 必中 Fire 分支

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [clear-confirm 倒计时内提交立即执行](../2026-09-01/clear-confirm-submit-fires-now.md)（71e7e1e，继承其 armed 可编辑 + Enter 提前提交语义，补齐 Shift+Tab 确认入口）
- [Shift+Tab 模式感知切换](../2026-09-01/act-shift-tab-mode-aware-switch.md)
- [clear-context 倒计时防护](../2026-08-29/clear-context-countdown-guard.md)
