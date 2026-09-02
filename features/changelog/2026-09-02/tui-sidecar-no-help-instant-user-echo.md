Commit: (working-tree, sidecar 提交即时回显)

# TUI `/sidecar`：去掉帮助文案，提交问题即时回显（消灭"提交后卡住"观感）

## 背景
两点体验问题：
1. **提交后卡一会**：`KeyAction::SidecarAsk` 只做 `try_send(SidecarCmd::Ask)`，UI 端零回显；actor 收到 Ask 后要先 `load_messages()` 快照 + `new_conv_from()` 建 conv，完成后才发 `SidecarStart`，UI 这才把空锚面板收养为有标题面板。这段空窗期内问题文本哪儿都不显示（`SidecarStart` 后也只进面板标题且截断 90 字符，嵌套 view 不回显）——观感即提交后没反应。
2. **帮助信息来自两处**（均为 test-pinned 常量）：`SIDECAR_ENTER_FLASH`（`/sidecar` 斜杠命令与 bare `/sidecar` 两处 flash）+ `SIDECAR_EMPTY_HINT`（空面板标题拼接）。帮助文案重复且提示冗余。

不会重复渲染：`SessionEvent` 无 user-echo 变体，主转录用户回显本就是 UI 本地 `push_user`；`run_sidecar_turn` 经 `SidecarChild` 只转发子内容帧，不会再次渲染问题——本地即时回显恰好补齐且只出现一次。

## 变更
- **`crates/tui/src/sidecar_ui.rs`**：删除 `SIDECAR_ENTER_FLASH`、`SIDECAR_EMPTY_HINT`（保留 `SIDECAR_BUSY_FLASH`，那是错误提示不是帮助）。新增 `echo_question(chat, question)`：向打开的面板（`chat.sidecar`）嵌套 view push `ChatBlock::User { rendered: markdown::render(question) }` + 空行 marker——与主转录 `push_user` 完全相同的正常对话渲染路径；面板关闭时 no-op。`SidecarStart` 原地收养面板保留嵌套 view，回显恰好存活一次。
- **`crates/tui/src/app.rs`**（`KeyAction::SidecarAsk`）：bare `/sidecar` 去掉 flash；带问题/追问路径在 `try_send` **Ok 后**才 `echo_question`（busy/失败路径不产生孤儿回显，Err 仍走 BUSY flash）。
- **`crates/tui/src/app_loop_actions.rs`**（`SlashAction::Sidecar`）：删掉 flash 赋值，保留 `enter_panel` + follow。
- **`crates/tui/src/app_loop.rs`**（`compute_display`）：空问题标题固定 `← [Ctrl+L] back | ⇲sidecar`（导航元素，非帮助文案）；有问题标题维持 `⇲sidecar {question}`。

**隔离不变量**（本变更依赖的前提）：sidecar 的 queued（actor 内部 backlog）与"steer"（有界 ask channel）区域完全隔离于父任务——`echo_question` 只写面板嵌套 view，不触碰父任务的 `queue_items`/`admit_st`/steer 路径，也不扰动 `blocks` 流式不变量（面板在 `ChatView::sidecar` 字段，不在 `blocks` 内）。

已知取舍：conv 构建失败（"sidecar unavailable"）时即时回显会保留但无答案——状态行已有报错，可接受。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 回显进面板嵌套 view（markdown User 块 + marker） | `echo_question_lands_in_placeholder_view` | `crates/tui/src/chat_tests/sidecar_fold.rs` |
| `SidecarStart` 收养后回显保留且不重复 | `echo_survives_start_adoption_without_duplication` | 同上 |
| follow-up 回显落同一面板块 | `followup_echo_joins_the_adopted_block` | 同上 |
| 无面板时回显 no-op | `echo_without_sidecar_block_is_a_noop` | 同上 |
| 空面板标题仅导航、无帮助文案 | `empty_placeholder_panel_title_is_nav_only` | `crates/tui/src/app_loop_tests/sidecar_display_tests.rs` |
| `/sidecar` 打开面板无 flash、Reset 照发 | `slash_action_sidecar_idle_opens_fresh_panel` | `crates/tui/src/app_loop_slash_action_tests.rs` |
| Esc/Ctrl+L 销毁面板（回归） | `esc_destroys_the_sidecar_panel` / `ctrl_l_destroys_the_sidecar_then_collapses_parent` | `crates/tui/src/app_helpers_tests/ctrl_l_tests.rs` |

## 回归（rules/02）
- `cargo test -p opencoder-tui`：27 个测试目标全绿（lib 1595 passed / 0 failed，含同期 `sidecar_stream_isolation` 流式隔离组）。
- `cargo test --workspace`：3926 passed / 0 failed。
- `cargo clippy -p opencoder-tui --all-targets`：无告警。
