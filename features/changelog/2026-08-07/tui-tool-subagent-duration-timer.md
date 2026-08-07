# TUI Tool/Subagent Duration Timer + Replay Bugfix

## 背景

Tool 和 Subagent 块在 live 会话中需要显示运行计时（running → 实时 warn 色
timer；done → 冻结 muted 色 timer，<1s 隐藏）。该功能已实现在 live 路径
（`ChatView::apply` 的 `ToolStart`/`ToolEnd` 事件），但 replay/`--continue`
路径存在 bug：replayed Tool 块使用 `elapsed_ms: None`（语义为"运行中"），
导致 `push_duration_span` 计算 `live = now_ms - 0 = epoch_ms`（~1.7 万亿
ms），在 resume 时渲染出荒谬的计时。

## 变更

### 1. Replay Tool 块计时 bug 修复

`crates/tui/src/session_ui/replay.rs` 两处 `ChatBlock::Tool` 构造点
（assistant ToolUse + fallback orphan-tool-result）将 `elapsed_ms: None`
改为 `elapsed_ms: Some(0)`。`Some(0)` 命中 `push_duration_span` 的 `< 1000`
守卫 → 提前 return，不推送 duration span，正确省略计时。

### 2. 回归测试

新增 `crates/tui/src/session_ui/replay_duration_tests.rs`（104 行，2 个测试）：
- `replayed_tool_block_omits_duration_span`：replay assistant ToolUse →
  断言 `elapsed_ms == Some(0)`；用 epoch-scale `now_ms` flatten → 断言不含
  garbage duration 字符串。
- `replayed_orphan_tool_result_omits_duration_span`：replay fallback
  orphan ToolResult → 断言 `elapsed_ms == Some(0)`。

### 3. 附带修复：keymap 特性编译阻塞

working tree 中 keymap 特性（`keymap.rs`、`keymap_menu/`、`KeyBindings`、
`SlashAction::ShortKey`）签名已改但调用方未同步更新，导致 TUI crate 无法
编译。修复内容：
- `app_loop.rs`：`dispatch_command` 增加 `keymap_menu` 参数；`ShortKey`
  arm 创建 `KeymapMenu::new(&config.keymap)`（消除 `KEYMAP_INFO` dead-code）。
- `keymap.rs`：`matches` 函数增加 Tab/BackTab SHIFT-optional 匹配（修复
  `match_tab_backtab_alt_tab` 测试：BackTab 归一化为 Tab+SHIFT 后，无
  SHIFT 的 binding 仍应匹配）。
- `app.rs` + 全部 `handle_key`/`pre_key_intercept`/`route_paste`/
  `render_frame` 调用方（含 8+ 测试文件）：传入新增参数。
- `keymap_menu/state.rs`：为 `KeymapMenu` 补充 `is_empty()`（clippy
  `len_without_is_empty`）。
- `app.rs`：keymap 菜单 `match` → `if let`（clippy `single_match`）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| replayed Tool 块省略计时 | `replayed_tool_block_omits_duration_span` | `crates/tui/src/session_ui/replay_duration_tests.rs` |
| replayed orphan ToolResult 省略计时 | `replayed_orphan_tool_result_omits_duration_span` | `crates/tui/src/session_ui/replay_duration_tests.rs` |
| Tab/BackTab SHIFT-optional 匹配 | `match_tab_backtab_alt_tab` | `crates/tui/src/keymap.rs` |

- 全量回归：`cargo test --workspace` → **2017 passed / 0 failed**
- TUI lib：`cargo test -p opencoder-tui --lib` → **1013 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished，零错误

## 备注

- 本次工作修复了 keymap 特性签名变更未同步调用方导致的编译阻塞。
  keymap 特性现已完整接线（详见 [keymap changelog](tui-keymap-short-key-rebindable.md)）。
- TUI lib 测试数从 review 基线 968 增至 1013（含 keymap 特性自带测试 +
  本次 2 个回归测试）。
