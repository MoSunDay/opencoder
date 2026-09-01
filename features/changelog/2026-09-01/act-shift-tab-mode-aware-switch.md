Commit: (working-tree)

# Shift+Tab 模式感知：act→plan 纯切换、plan→arm 清上下文交接

## 背景

Shift+Tab 原语义是「无条件 arm 清上下文倒计时」，act 模式下按键即进入破坏性
折叠防护，与 Ctrl+T 的非破坏切换并存后语义冗余且易误触。用户要求：act 模式
Shift+Tab 直接切回 plan（上下文保留），plan 模式 Shift+Tab 才是「保留计划并
交给 act 执行」的倒计时防护入口。

## 实现

- **`crates/tui/src/key_handler.rs`**：BackTab 分支按 `agent` 分流——`act` 产
  `KeyAction::SwitchAgent("plan")`（与 Ctrl+T 同路，复用 dispatch_mode_switch
  的 busy gate / 持久化 / 状态反馈）；其余（plan）产 `ArmClearConfirm`，走
  `/act_clear_context` 同一倒计时防护。
- **回撤载荷补全（评审缺口修复）**：`ArmClearConfirm { rest, draft }`——BackTab
  分支清空 composer 前捕获原始未 trim 草稿为 `draft`，`app.rs` 消费处透传
  `engage` 的 `restore_draft`。此前 `rest` 只携带 trimmed 尾参、`restore_draft`
  恒为 `None`：Esc 回撤后草稿随 arm 一并丢弃，与 key_handler 注释、
  clear_confirm 模块 doc、测试注释三处「Esc 恢复 draft」的声明相悖（slash 文本
  命令入口经 `maybe_arm` 传 `Some(text)` 无此问题，故无组合测试覆盖而漏网）。
  修复后 Esc 回撤把 arming 前的输入逐字节还原（trim 只影响复合尾部，不影响回撤）。
- **`keymap_menu/help.rs`**：帮助文案同步为模式感知语义；负向守卫
  `!HELP.contains("Ctrl+Shift+Tab")`（退役绑定见
  [retire-ctrl-shift-tab-binding](retire-ctrl-shift-tab-binding.md)）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| act 模式 BackTab 纯切 plan（idle/running 双态、草稿不动） | `backtab_in_act_mode_switches_to_plan` | `crates/tui/src/key_handler_running_mode_tests.rs` |
| plan 模式 BackTab arm 倒计时 + rest/draft 载荷分离（raw 与 trimmed） | `backtab_in_plan_mode_arms_clear_context_confirm` | `crates/tui/src/key_handler_running_mode_tests.rs` |
| BackTab-arm→Esc 回撤逐字节还原草稿（handle_key→arm→intercept 全链路） | `backtab_arm_then_esc_restores_the_raw_draft` | `crates/tui/src/key_handler_running_mode_tests.rs` |
| armed 期间 Esc 恢复 restore_draft、丢 arm | `intercept_esc_cancels_and_restores_the_draft` | `crates/tui/src/clear_confirm.rs` |
| fire 时 running 原文入队不起新 turn | `fired_guard_queues_compound_when_running` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |
| plan→act 折叠真实执行链路 | `crates/tui/tests/act_clear_context_fold.rs` 全套（5 tests） | `crates/tui/tests/act_clear_context_fold.rs` |

## 回归证据

- `cargo test -p opencoder-tui --lib` → **1524 passed / 0 failed**（最终收敛树，
  含上表 5 例具名测试逐项 ok；期间并行流重构 handle_key 签名后复验仍绿）
- `cargo test -p opencoder-tui --test act_clear_context_fold` → 5 passed / 0 failed
- 收敛后全量门禁（b243c2b，与 HEAD 5ed895a 功能等价）：`cargo test
  --workspace --no-fail-fast` 244 target → 2280 passed / 8 failed，8 例均为
  负载饥饿或已修复项（明细见
  [retire-ctrl-shift-tab-binding](retire-ctrl-shift-tab-binding.md) 全量回归
  节）；失败 target 单线程复跑全绿，tui lib 补跑 1518 / 0，上表 5 例具名
  测试逐项 ok。全量口径 0 failed 闭环。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [clear-confirm 倒计时内提交立即执行](clear-confirm-submit-fires-now.md)
- [act/plan 状态切换与执行交接](act-plan-switch-and-clear-handoff.md)
- [退役 ctrl+shift+tab 绑定](retire-ctrl-shift-tab-binding.md)
