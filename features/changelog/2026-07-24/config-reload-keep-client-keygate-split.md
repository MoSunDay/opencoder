Commit: (working-tree, pre-initial-commit)

# fix(tui,session): 配置热重载失败时保留旧 client，subagent 视图屏蔽模式切换键，拆分 runner/chat 模块

## 背景

### 配置热重载静默吞错
`/config` reload 路径（`worker.rs` 与 `app_loop.rs::handle_model_outcome`）此前用
`if let Ok(ep) = ... { if let Ok(client) = ... { ... } }` 形式：当
`resolve_endpoint()` 或 `ChatClient::new()` 失败时，错误被静默丢弃，旧的 model/client
状态与新落盘的 config 不一致——用户看到 `/config` 成功、model 字段却仍是旧值，
且无任何提示。

### subagent 视图仍可触发 act/plan 切换
`handle_key` 在 `2026-07-24/tui-plan-act-handoff` 中已加入 `input_disabled`
early-return（进入 subagent-focus 视图后禁用文本输入）。但 Alt+Tab / Ctrl+Shift+Tab
两个 act↔plan 模式切换分支仍排在 early-return **之前**，导致在只读 subagent
视图里按这些键仍会切走父 agent——破坏「只读查看」语义。

### runner.rs / chat.rs 过长
`crates/session/src/runner.rs` 已达 1222 行，`crates/tui/src/chat.rs` 内的类型定义
挤占了主逻辑。需按职责拆分以满足 800 行限制。

## 变更

### 配置热重载 keep-client fallback
- **`crates/session/src/lib.rs`**：新增 `pub fn apply_config_reload_keep_client(&mut self, new_cfg)`，
  只更新 `model` 与 `config` 字段，保留现有 `client`，使运行中会话在 client 构建失败时
  仍与落盘 config 的 model 字段保持一致。
- **`crates/tui/src/worker.rs::process_cmd`** `ReloadConfig` 分支：由 `if let Ok` 改为
  `match`，新增两条 `Err` 路径：
  - `resolve_endpoint()` 失败 → `apply_config_reload_keep_client` + 转发 `SessionEvent::Error`，
    消息含新 model 名与失败原因。
  - `ChatClient::new()` 失败 → 同上 keep-client fallback + Error 事件。
- **`crates/tui/src/app_loop.rs::handle_model_outcome`**：同样改为 `match`，两条 `Err`
  路径在 UI 推入红色 `[/config] ... failed ... keeps previous client` marker 提示行。

### subagent 视图屏蔽模式切换键
- **`crates/tui/src/key_handler.rs`**：将 Alt+Tab / Ctrl+Shift+Tab 两个 act↔plan
  切换分支**移动到** `input_disabled` early-return 之后。效果：input 禁用（subagent-focus）
  时这些键返回 `KeyAction::None`，不再切走父 agent；input 可用时行为不变。

### 模块拆分（纯重构，无行为变化）
- **`crates/session/src/runner.rs`**（1222 行）拆分为：
  - `runner/mod.rs`（600 行）—— drain 主循环 + steer/queue + doom-loop 守卫。
  - `runner/event.rs`（343 行）—— session event 处理。
  - `runner/subagent.rs`（294 行）—— subagent 调度与追踪。
- **`crates/tui/src/chat.rs`**：`ChatBlock` / `ChatView` / `ThinkingHeader` /
  `SubagentHeader` / `SPINNER` / `TOOL_OUTPUT_LINES` 抽取到
  `crates/tui/src/chat_types.rs`（111 行），`chat.rs` 经 `#[path]` 重导出，
  全 crate `use` 路径不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| reload 成功正常换 model | `reload_config_success_swaps_model` | `crates/tui/src/worker.rs` |
| reload 坏 proxy 保留旧 client 并报错 | `reload_config_bad_proxy_keeps_client_and_emits_error` | `crates/tui/src/worker.rs` |
| subagent 视图屏蔽 Alt+Tab | `handle_key_disabled_blocks_alt_tab` | `crates/tui/src/key_handler.rs` |
| subagent 视图屏蔽 Ctrl+Shift+Tab | `handle_key_disabled_blocks_ctrl_shift_tab` | `crates/tui/src/key_handler.rs` |

- 全量回归：`cargo test --workspace` → 由用户预跑确认全绿。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 由用户预跑确认零警告。
- 行数：`runner/mod.rs` 600 ≤ 800；`runner/event.rs` 343 ≤ 800；`runner/subagent.rs` 294 ≤ 800；`chat_types.rs` 111 ≤ 400。

## Impact Surface
- **用户可感知**：`/config` reload 因 endpoint 解析或 client 构建失败时，model 字段
  仍更新为落盘值并显式提示错误（旧 client 保留）；进入 subagent 视图后 Alt+Tab /
  Ctrl+Shift+Tab 不再误切父 agent。
- **不影响**：`Store` / `ChatStream` 抽象边界、drain 语义、HTTP/SSE、CLI 子命令、
  session 持久化路径均未改。

## Related Docs
- [agents/session/index.md](../../agents/session/index.md)
- [agents/tui](../../agents/tui/index.md)（若有）
- [2026-07-24/tui-plan-act-handoff-defer-run-timer.md](tui-plan-act-handoff-defer-run-timer.md)（subagent-focus input_disabled 的前序变更）
