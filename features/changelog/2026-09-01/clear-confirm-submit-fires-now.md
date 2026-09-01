Commit: 62ad7f0 (working-tree, clear-confirm 倒计时内提交立即执行)

# clear-confirm 倒计时内提交立即执行（输入框保持可编辑，不必等满 5s）

## 背景

`/act_clear_context`（Shift+Tab / 文本命令）arm 5s 倒计时防护后，armed 期间全部按键被吞——除了裸 Enter（提前执行）之外用户什么也做不了：想补一句「执行时顺便做什么」再提交，键入是 inert 的，文本直接丢失，只能干等 5s 让窗口自动 fire，或 Esc 回撤重来。用户要求：计入倒计时后再执行一次提交就立刻执行，不必非要等 5s。

## 实现

- **`crates/tui/src/clear_confirm.rs`**：
  - `intercept` 从「吞掉全部按键」改为「composer 保持可编辑」：`Char`（经 `composer::insert_char`，undo 折叠快照）、`Backspace`、`Left`/`Right` 照常编辑；`Shift|Alt+Enter` 仍插换行不触发 fire（与 live composer 一致）；`Alt|Ctrl` 组合键维持 inert（tmux 转义幽灵字符守卫）；其余键（Up/Tab 等）继续吞掉。
  - 新增纯函数 `merge_typed`：提交时把倒计时内键入的文本折入复合尾部——重键入的 clear 命令（两条拼写经 `head_rest` 判定）其尾部**取代** armed rest（最新意图胜出），普通文本**追加**到 armed rest 之后，空白输入不动 arm。
  - 模块 doc 同步；`CLEAR_CONFIRM_WINDOW_MS`/状态机/tick/回撤语义零变更。
- **`app_loop_actions.rs::handle_confirm_key`**：`ConfirmFlow::Fire` 臂先 `merge_typed`（并清空 composer + undo reset）再 `fire_clear_confirm`——提交即确认，立即执行；tick 到点自动 fire 路径不变（未提交的键入文本不折入，留在输入框由用户后续处置）。idle 直接开 turn、running 原文排队不变。
- **`keymap_menu/help.rs`**：Shift+Tab 帮助文案改为「先倒计时确认：Esc 回撤，提交（Enter）立即执行；倒计时内可继续输入，提交的输入并入附加需求」。

## 有意保留

- transcript 的 `[clear]` 标记与倒计时 chip 不加按键提示（2026-08-30 收敛决策，避免噪音）。
- `Esc` 回撤仍恢复 `restore_draft`（arm 时被吞的草稿）；倒计时内键入且未提交的文本随回撤按既有语义处理（有草稿则被草稿覆盖）。
- Tab 在 armed 期间保持 inert（它不是确认语义；「提交」指 Enter）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| armed 期间编辑键存活、其余键/Alt/Ctrl 组合 inert | `intercept_editing_keys_stay_live_others_inert` | `crates/tui/src/clear_confirm.rs` |
| Shift+Enter 插换行不触发 fire | `intercept_shift_enter_inserts_newline_instead_of_firing` | `crates/tui/src/clear_confirm.rs` |
| merge_typed 追加/取代/空白不动 arm | `merge_typed_appends_supersedes_and_ignores_blank` | `crates/tui/src/clear_confirm.rs` |
| 倒计时内键入 + Enter 立即 fire 且复合尾部合并、composer 清空 | `enter_with_typed_text_fires_merged_rest_now` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |
| 重键入 clear 命令取代 armed rest | `retyped_clear_command_supersedes_armed_rest` | `crates/tui/src/app_loop_dispatch_cmd_tests/act_clear.rs` |

- 既有测试不弱化：`intercept_enter_fires_and_leaves_arm_for_caller`（裸 Enter 仍提前 fire）、`esc_cancel_drops_countdown_chip`、`fired_guard_*` idle/running 双路径全部保留。
- 全量回归：`cargo test --workspace` → 245 套件 / 3790 passed / 0 failed（TEST-EXIT=0；同轮曾出现 `tmux_bar::tests::hide_returns_none_outside_tmux` 并行环境竞态 flake——进程级 `TMUX` env 在 tmux 会话内被并行测试改写，solo 复跑 PASS，与本轮改动无关）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（CLIPPY-EXIT=0）

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [clear-context 倒计时防护](../2026-08-29/clear-context-countdown-guard.md)（38cbd84，本轮修改其「armed 期间吞全部键」语义）
- [arm 后提示收敛](../2026-08-30/clear-confirm-copy-collapse.md)（0108e5c）

## 本地部署

- `cargo build --release` 成功（`CARGO_TARGET_DIR=/data00/rust-build/cargo/default`）。
  首次部署为提交前构建，版本 `opencoder 0.1.0 (62ad7f0-dirty)`（SHA-256 `f08d1709…`）；
  落 commit `71e7e1e` 后从干净树重建，版本串收敛为 `opencoder 0.1.0 (71e7e1e)`。
- 最终生效的 release 产物已原子替换 PATH 首选项 `/root/.local/bin/opencoder`；部署后 SHA-256 为
  `e3accf1e4213df04fb193981a848f0fbfbbed5c6bf4f9e8b9151f79d4072f26d`，与构建产物一致，
  可由 commit `71e7e1e` 复现。
- 原二进制保留在
  `/root/.local/bin/opencoder.backup-before-clear-confirm-20260901`，可用于回退；
  部署时已在运行的进程仍使用旧映像，重新启动后加载新版本。
