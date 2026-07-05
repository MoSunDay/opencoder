Commit: (working-tree, pre-initial-commit)

# 迭代四：TUI 交互重设计 + mid-tool 硬中止

## Context
迭代三把 TUI 拉齐到 4-region 布局，但仍存若干交互缺陷：① Ctrl+T 切 plan/act 是按 `running` 取值（idle 时永远进 plan，无法回 act），且运行中被 `if !running` 守卫挡死；② 顶部 header 占整行而底部 status 信息稀疏；③ composer 边框常驻大段提示串「Enter=send, Ctrl+T=plan/act …」；④ body 的 `render_body` 恒以 `max_scroll` 渲染——`scroll`/`follow` 变量实际未驱动视图，PageUp/Ctrl+U/D 视觉无效，且无滚动条；⑤ 运行中无动态反馈；⑥ Help 只能 Ctrl+H 关、Esc 不能关；⑦ 无任务中止能力（cancel 仅 web turn 边界）。本迭代修复全部 7 项。

## Change Summary
- **Ctrl+T 双向切换（修 bug）**：`handle_key` 改按**当前 agent** 取反（`plan`→`act` / 否则→`plan`），并去掉运行中不可切的守卫。
- **布局合并**：删掉顶部 header 行，将其内容（opencode / model / `[agent]` / dir / ctx%）并入底部 status；status 去掉 `idle`/`running` 静态词与 `Ctrl+H=help` 提示。Layout 由 4-region 收为 body(Min) / composer(3) / status(1)。
- **composer 去提示**：移除边框 title 中的「Enter=send …」占位串。
- **滚动条 + 自动跟随 + 鼠标**：`render_body` 改用真实 scroll 偏移并叠加 `Scrollbar`（`ScrollbarOrientation::VerticalRight`）；跟随态在 composer 右上角显示「跟随中…」，上滚后变为可点击「↓」，点击跳底。`EnableMouseCapture` 后 `Event::Mouse` 经 `MouseHits { jump_btn, body }` 命中测试：左键点 ↓ → `follow=true`；滚轮在 body 区上滚 → `follow=false` + 减偏移，下滚 → 增偏移、触底自动恢复跟随。PageDown 仍为键盘跳底回退。
- **运行中动画**：主 `select!` 新增 `tokio::time::interval(300ms)` tick，仅 `running` 时推进帧计数；status 以 braille spinner（`SPINNER[10]`）+ status 文本显示。
- **Esc 关 Help**：`show_help` 打开时单次 Esc 优先关 Help。
- **双击 Esc 硬中止**：TUI 建 `CancellationToken` 经 `session.with_cancel(...)` 挂载；350ms 内两次 Esc 且运行中 → `cancel.cancel()`。runner 侧 `run_one_llm_call` 流接收循环与 `execute_call` 工具 `.await` 均包 `tokio::select! { biased; _ = await_cancel(..) => .., .. }`；bash 已 `kill_on_drop(true)`，丢弃 future 即杀子进程；子 agent 复用父 token。
- **Ctrl+D 退出**：原 Ctrl+D（下滚）改为 `KeyAction::Quit`；`Ctrl+C`/`Ctrl+D` 并列退出。滚动改由 PageUp/Down + 滚动条承担。
- keybind help 同步重写。

## Impact Surface
- 重写：`crates/tui/src/app.rs`（472→646 行，仍 ≤800 限）、`crates/tui/src/keybind.rs`。
- 改动：`crates/session/src/runner.rs`（新增 `cancelled` / `await_cancel` 辅助；run_one_llm_call 与 execute_call 改 select!；run_loop 工具循环加 `if cancelled(session) { break; }`；`run_subagent` 透传 `child.cancel`）。
- 新增依赖：`opencode-tui` 加 `tokio-util.workspace = true`（CancellationToken）。
- 新增测试：`crates/session/tests/hard_abort.rs`（cancel 中止运行中 bash `sleep`，sub-3s 返回 + `Status(interrupted)`）。
- 不变：`SessionEvent` 枚举、`Store`/`ChatStream` trait、web/CLI 入口、持久化路径。

## Verification
- `cargo build --bin opencoder`：dev profile 无 warning。
- `cargo clippy -p opencode-tui -p opencode-session --tests`：全绿。
- `cargo test --workspace`：全过（含既有 58 + 新增 hard_abort；web 的 turn 边界 interrupt 测试仍绿，证明两套 cancel 共存）。
- PTY 冒烟（`script` + `timeout 1.5 opencode tui`）：进程进入渲染循环、运行满 1.5s 被 timeout 杀（exit 124），无 panic / 无 stderr。

## Notes / Compatibility
- 硬中止为「mid-tool 软停」：cancel 在 select! 命中后立即丢弃当前 future（bash 子进程被杀），但**已完成流式输出的部分文本不回滚**，且会被记为一条空 assistant 消息（cancel 命中 LLM 流时）或 `interrupted` 工具结果。可接受。
- 鼠标捕获会改变终端「鼠标选中文本复制」行为（部分终端需按住 Shift 选中）——为换取可点击 ↓ 的取舍。
- 滚动条/scroll 仍以「行」为单位，长行 wrap 时 thumb 位置为近似（与迭代三一致的已知精度）。
- ratatui alt-screen 下被 SIGTERM 杀进程不会刷新最终帧到 typescript——崩溃检测靠 exit code 与 stderr，非靠帧抓取。

## Related Docs
- [agents/session](../../../agents/session/index.md) —— cancel 语义（turn 边界 + mid-tool）、run_loop。
- [迭代三 TUI changelog](./iteration3-tui-overhaul.md) —— 4-region 基线与 scrollback/subagent/steer 来源。
