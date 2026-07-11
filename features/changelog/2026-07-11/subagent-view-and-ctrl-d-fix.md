Commit: (working-tree, pre-initial-commit)

# subagent 点击直入视角 + Ctrl+D 在 Kitty 键盘协议下失效修复

## 背景
1. subagent 点击行为与 thinking 一致（先内联展开，再点才进入 ctx 视图），用户体验混淆——用户期望点击直接切换到 subagent 视角看任务和 context 统计。
2. 在支持 Kitty 键盘协议的终端（Kitty / WezTerm / Ghostty 等）上 Ctrl+D 无法退出 opencoder——按了没反应。

## 变更

### subagent 点击直入视角（不再内联展开）
- **`crates/tui/src/chat.rs`**：
  - `ChatBlock::Subagent` 移除 `collapsed` 字段——不再支持内联展开/折叠。
  - `ChatView` 新增 `context_used: u64` 字段，`apply()` 内部调 `track_context()` 逐事件累加自身 transcript token（不递归 SubagentChild——子 view 的 `apply()` 独立维护自己的计数）。
  - `flatten()` Subagent 分支重写：始终单行表头 `⇲ subagent [kind] prompt [mark status, N tools] [→ view]`，running=黄● / done=绿✔ / failed=红✘；done 时追加 summary。
  - `thinking_headers()` / `subagent_headers()` Subagent 分支简化为恒定 1 行。
  - 移除 `toggle_subagent_at()` 方法。
- **`crates/tui/src/app.rs`**：
  - 点击处理简化：单击直接 `subagent_focus = Some(block_idx)`，不再判断 collapsed / 调 toggle。
  - render 阶段 `display_ctx` / `display_sys`：subagent_focus 时传子 view 的 `context_used`（sys_tokens=0），否则传父级 `context_used` + `sys_tokens`——状态栏 ctx 仪表反映当前视角。

### Ctrl+D / Ctrl+C 在 Kitty 键盘协议下恢复退出
- **根因**：`DISAMBIGUATE_ESCAPE_CODES`（`app.rs:76`）启用后，crossterm 0.28.1 将 Ctrl+D 报为 `Char('\u{4}')` + CONTROL（而非 `Char('d')` + CONTROL）。CONTROL 块只匹配 `Char('c') | Char('d')`，`\u{4}` 落入 `_ => KeyAction::None` 被静默吞掉；底部 raw-EOT fallback（在非 CONTROL 的 `Char(c)` 分支）永远不可达。
- **修复**：5 个处理器的 CONTROL 块同时匹配原始控制字符 `Char('\u{3}') | Char('\u{4}')`：
  - `key_handler.rs:63`（主分发器）
  - `command.rs:148`（`/` 命令面板）
  - `menu.rs:38`（`$` skill 菜单）
  - `task.rs:70`（`/task` 会话选择器）
  - `model_menu/state.rs:214`（`/model` 配置）
- **额外防御**：`app.rs` 事件臂 `events.next()` 返回 `None`/`Err`（流关闭）时直接 `UiCmd::Quit` + break，防止死流忙循环。

### 统一 context 统计（消除死数据 + 子视角 sys_tokens）
- **`crates/tui/src/chat.rs`**：`ChatView::track_context` 增加 `SubagentChild` 递归——父 view 的 `context_used` 含全部后代 token，每个子 view 独立维护自身子树。
- **`crates/tui/src/app.rs`**：移除独立 `context_used` 变量和 `track_context` 函数——直接读写 `chat.context_used`（由 `ChatView::apply()` 内部维护），消除父 ChatView 上的死数据。
- **`crates/tui/src/session_ui.rs`**：`SessionUiState` 移除 `context_used` 字段（由克隆的 `ChatView.context_used` 携带），`snapshot()` 参数对应精简。
- **子视角 sys_tokens**：进入 subagent 视图时调 `sys_tokens_for(kind, workdir, None)` 缓存到 `subagent_sys`（不每帧重算），使子视角 ctx% 包含系统提示 token。

### 代码风格
- `app.rs` `match ev` 块统一缩进（移除旧 `if let` 包装遗留的 4 空格偏移）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Kitty Ctrl+D（`\u{4}` + CONTROL）退出 | `kitty_ctrl_d_quits` | `crates/tui/src/app_tests.rs` |
| Kitty Ctrl+C（`\u{3}` + CONTROL）退出 | `kitty_ctrl_c_quits` | `crates/tui/src/app_tests.rs` |
| subagent 单击直入视角、子 view context 独立 | `subagent_events_render` | `crates/tui/src/chat.rs`（更新） |
| 既有 Ctrl+D / raw EOT / Ctrl+C / raw ETX 退出 | `ctrl_d_quits` / `raw_eot_quits` / `ctrl_c_quits` / `raw_etx_quits` | `crates/tui/src/app_tests.rs` |
| session_ui snapshot/restore 无 context_used 字段 | `snapshot_captures_all_fields` / `roundtrip_snapshot_then_compare` | `crates/tui/src/session_ui.rs`（更新） |

- 全量回归：`cargo test --workspace` → 0 failed（TUI 110/110）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Impact Surface
- TUI 用户在 Kitty 键盘协议终端上 Ctrl+D / Ctrl+C 恢复正常退出。
- subagent 点击交互变更：单击直接进入子视角（无内联展开步骤），状态栏显示子会话 ctx 统计（含 sys_tokens 估算）。
- 不影响 CLI / Web / session / store 层。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
