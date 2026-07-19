# 去掉 steer 提交时的两处用户可见噪音文案

## 背景

用户反馈两处展示型文案造成视觉干扰：

1. **右下角状态行**：每次 drain 主循环在 turn 边界提升 queue 中的 steer 时，会发射一条 `SessionEvent::Status("steer promoted (N new input(s))")`，TUI 把它渲染到右下角状态区。频繁会话里这条文案反复闪现。
2. **steer `>` 点击 marker**：在会话运行中点击某条 steer 的 `>` 提交按钮时，TUI 会在 chat 区推入一条黄色 marker 行 `"[interrupted] resuming…"`，提示「正在中断当前 turn 并从提升的 steer 续跑」。

两条文案都是纯展示，**与核心中断 / 提交 / 提升逻辑无关**。需求：删除这两处文案，保留底层 `claim_steers` / `record` / `SteerConsumed` / `cancel.cancel()` / `cancelled` / `drain_pending` 全部控制流不变。

## 变更

### `crates/session/src/runner.rs`（`run_loop`，~L255）

删除 steer 提升块末尾的 `on_event(SessionEvent::Status(format!("steer promoted ({} new input(s))", steer_prompts.len())))`。同块的 `claim_steers`、`session.record(m)`、`on_event(SessionEvent::SteerConsumed { seq: *seq })` 全部保留——提升语义与事件契约不变。

### `crates/tui/src/app.rs`（`run_app`，`MouseOutcome::SteerSubmit` 分支，~L872）

删除 `chat.push_marker(Line::from(Span::styled("[interrupted] resuming\u{2026}", Style::default().fg(Color::Yellow))))`。同分支的 `cancel.cancel()`、`cancelled = true;`、`drain_pending = true;` 全部保留——点击 steer `>` 仍会中断当前 turn 并在下一边界消费提升的 steer，只是不再画那行黄色提示。

净改动：`-8 / +0` 行，纯删除展示型代码，无控制流 / 数据形状变化。同文件另一处 `KeyAction::Cancel` 分支的 `"[interrupted] stopping…"` marker（与硬中断相关，语义不同）保留不动。

## 测试覆盖

本变更**无新增测试**——纯删除展示型文案，无新行为 / 新分支 / 新数据形状可测。底层保留逻辑仍由现有测试覆盖：

| 保留逻辑 | 覆盖测试 | 文件 |
|----------|----------|------|
| `claim_steers` / `record` / `SessionEvent::SteerConsumed { seq }` 提升语义 | `steer_consumed_carries_pk_seq_not_admitted_seq` | `crates/session/tests/steer_followup.rs` |
| 多条 steer 各自携带 pk seq、按序消费 | `multiple_steers_consumed_each_carries_distinct_pk_seq` | `crates/session/tests/steer_followup.rs` |
| `MouseOutcome::SteerSubmit` 点击检测（驱动 deleted-marker 分支的前置条件） | `submit_btn_returns_steer_submit` | `crates/tui/src/app_helpers.rs` |

上述测试均**不**断言被删除的 `"steer promoted"` 状态文案或 `"[interrupted] resuming…"` marker 文本（已 grep 全仓库确认：删除后 `crates/` 内对这两段字符串零引用），故删除是行为保持的、不破坏任何既有断言。

> 诚实说明：`run_app` 内层语句（`cancel.cancel(); cancelled=true; drain_pending=true;`）属 live 终端事件循环，未被单测直接驱动；删除安全性的依据是「无任何断言引用被删 marker 文本 + 周边控制流未动」，而非新测试。

## Gate（当次实跑取证）

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 584 passed / 0 failed / 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |

> 计数说明：本变更新增 **0** 个测试。工作树同时含一处**范围外、未提交**的无关 feature（`crates/tui/src/menu.rs`：skill 菜单 Tab 键确认，+4 测试），计入上述 584 总数，但不属本次变更范围，需独立提交。纯删除部分对应的回归基线无下降（无任何 `#[test]` 被删 / 新增 `#[ignore]` / 弱断言替换）。
