Commit: (working-tree, sidecar 面板注册化 + 销毁语义收敛 + review 收口：Reset 清 backlog)

# TUI `/sidecar` 收敛：slash 注册、统一入口、退出即销毁、transcript 零痕

## 背景
上一版 sidecar（见 [tui-sidecar-question-actor](tui-sidecar-question-actor.md)）
存在三处语义债：
1. **注册分裂**：`/sidecar` 未进 `command.rs` 的 `COMMANDS`/`SlashAction`，仅靠
   自由文本拦截（`parse_sidecar_question`）和裸 key 分支进入，弹出菜单（popup）
   看不到该命令；
2. **入口双重语义**：已聚焦时再发 `/sidecar`（bare）只闪 hint（「已在面板中」），
   面板里残留上一段对话；无「重开新面板」的路径；
3. **退出即隐藏**：ESC/Ctrl+L 仅 `sidecar_focus = false`，块留在 transcript、
   actor 的对话继续存活——旧侧车内容随 re-focus 回来（陈旧快照），且折叠态
   头行永远留在主 transcript。

## 变更
### 入口统一（Step 1-2）
- **`command.rs`**：`SlashAction::Sidecar` 变体 + `parse("sidecar")` 精确匹配
  （`/sidecar <q>` 仍由 composer 拦截，`parse` 只见 bare token）+ `dispatch`
  + `COMMANDS` 注册（popup 可见）；HELP 文案同步（`keymap_menu/help.rs`）。
- **`app_loop_actions.rs`**：`SlashAction::Sidecar` 分支**不受 running 门禁**
  （旁路语义：不碰 steer/queue/prompt），动作 = `enter_panel` + `follow=true`
  + `SIDECAR_ENTER_FLASH`。
- **`app.rs`**：bare `/sidecar` 分支改为「进入空面板」（原来只是重聚焦闪 hint）。

### 销毁语义（Step 3，核心）
- **`sidecar_ui.rs`** 重写 actor 命令面：`Sender<String>` → `Sender<SidecarCmd>`
  （`Ask(String)`/`Reset`）。回合以 `tokio::spawn` 跑，`select!` 竞速回合与命令；
  `Reset` = `handle.abort()` + join（usage 收集器定格）→ 丢弃 `SidecarConv` →
  下一 `Ask` 经 `new_conv_from` 用**新 store 快照**重建（修掉陈旧快照问题）；
  mid-turn Ask 进本地 backlog 串行补跑；**`Reset` 连 backlog 一并丢弃**（销毁
  即销毁一切：排队问题绝不重建对话、绝不烧 token、零 usage 入账——否则 ESC
  后排队的 follow-up 会继续消耗并在主任务侧留下无 UI 解释的成本）。被中止回合
  已产出的裸 `LlmUsage` 照常持久化（部分成本入账，web replay 对账不亏）。
- **进入/退出对称**：`enter_panel`（`/sidecar` 与 free-text 双入口）与
  `exit_panel`（ESC/Ctrl+L）都发 `Reset` + `sidecar::purge`（块全删 + 失焦）；
  进入再推一个空占位块（id 空、question 空）承接面板。
- **`chat_sidecar.rs`**：Start 帧原位**收养**占位块（不推第二块）；面板关闭时
  迟到的 Start 帧吞弃（防僵尸块 + 抢焦点）；`collapse_focused` 删除（Ctrl+L
  改为销毁，折叠失去意义）；新增 `purge()`。
- 不动 session 侧（`runner/sidecar.rs`、零持久化门、`worker/tests_sidecar.rs`
  token 对账）——下游契约零变更。

### transcript 零痕（Step 4）
- **`chat.rs`** flatten Sidecar 臂 → 0 行；**`chat_headers.rs`** 计头 0 行：
  侧车 Q/A 不再以任何形式出现在主 transcript（聚焦 body 由 `compute_display`
  换入）。
- **`app_loop.rs`**：空占位面板标题给 `SIDECAR_EMPTY_HINT`（有 question 后
  恢复回显）。
- Flash 常量收敛：删 `SIDECAR_FOCUSED_FLASH`/`SIDECAR_HINT_FLASH`，新增
  `SIDECAR_ENTER_FLASH`。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| popup 注册/parse 精确匹配 | `parse_sidecar`、`dispatch_sidecar` | `command.rs` |
| SlashAction idle/running 双分支（Reset 发出、占位块、flash、不扰主任务） | `slash_action_sidecar_idle_opens_fresh_panel`、`slash_action_sidecar_running_still_opens_panel` | `app_loop_slash_action_tests.rs` |
| ESC 销毁（Reset + purge + 草稿不动）；Ctrl+L 销毁后仍折叠父视图 | `esc_destroys_the_sidecar_panel`、`ctrl_l_destroys_the_sidecar_then_collapses_parent` | `app_helpers_tests/ctrl_l_tests.rs` |
| actor：idle Reset 重建（无旧 Q/A 泄漏）、mid-turn abort（无 Turn 帧、无 usage、actor 存活、重建无污染） | `sidecar_reset_idle_destroys_conversation_next_ask_rebuilds_fresh`、`sidecar_reset_aborts_inflight_turn_no_content_frames` | `sidecar_ui_tests.rs` |
| actor：Reset 丢弃 backlog（在飞 abort + 排队问题不执行、零 LLM 调用、零 usage、actor 存活重建） | `sidecar_reset_discards_backlogged_follow_ups`（缺陷注入验证：移除 `backlog.clear()` 即红） | `sidecar_ui_tests.rs` |
| 折叠：占位原位采用/关面板吞 Start/purge/flatten 零行 | `start_adopts_the_placeholder_in_place`、`start_with_closed_panel_is_swallowed`、`purge_removes_every_sidecar_block_and_the_focus`、`flatten_emits_zero_lines_for_sidecar_blocks` | `chat_tests/sidecar_fold.rs` |
| 显示：空面板 hint 标题、销毁后父 body 还原零痕 | `empty_placeholder_panel_shows_the_enter_hint`、`unfocused_sidecar_restores_the_parent_body` | `app_loop_tests/sidecar_display_tests.rs` |
| 行数账目对齐 flatten（Sidecar 臂 0 行） | `line_accounting_matches` | `chat_tests/line_accounting.rs` |

## 全量回归
- `cargo test -p opencoder-tui` → lib 1566 passed / 0 failed（含
  `sidecar_reset_discards_backlogged_follow_ups`），integration 全部套件绿；
  `cargo clippy -p opencoder-tui --all-targets` 零警告。
- `cargo test --workspace --no-fail-fast` → 246 套件 ok；唯三红为
  `crates/session/tests/` 的 `skill_body_injection`/`skill_mid_run`/
  `skill_tail_cleared_after_run_end`——其它会话并行 WIP（工作树
  `skill_context.rs` 未提交改动）所致，与本变更（crates/tui + 文档，零
  session 侧 diff）无关；`opencoder-llm` retry 两测试本轮未红。
- 行数：`sidecar_ui.rs` ~300 行（≤400 上限内）。

## Impact Surface
- TUI 面板入口/退出交互、popup 命令清单；session 侧与持久化契约零变更。
- Web/其它前端零影响（`Sidecar*` 帧仍不落库；裸 `LlmUsage` 对账不变）。

## Related Docs
- [TUI sidecar 初版](tui-sidecar-question-actor.md)
- [agents/tui](../../../agents/tui/index.md)
