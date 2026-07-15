# 无损 SSE 事件回放（store schema v2 迁移 + SessionEvent 单一真相源）+ TUI t+Tab/置顶按钮

## 背景

`SessionEvent` 有 16 个细粒度变体，但持久化层只有 12 变体的粗粒度 `EventKind`。回放路径（`GET /events` 的 `event_kind_str`）只能把 8 个变体折叠成通用名（`status`/`text_delta`/`compaction` 等），导致 **live 广播与回放的事件名不一致**——web/SSE 客户端回放历史会话时丢失 `reasoning_delta`/`subagent_*`/`plan_handoff` 等细粒度信息。同时 TUI 驱动的会话此前**根本不持久化事件**到 `session_events`，web 客户端无法回放 TUI 跑过的 session。

附带两项 TUI UX 改进：① `t+Tab` 和弦在不触发 plan→act handoff（不清空 transcript）的前提下切换模式；② 正文顶部边框新增 `⬆` 置顶跳转按钮。

## 变更

### 无损回放核心
### `crates/session/src/runner.rs`
- `SessionEvent` 新增**单一真相源**三方法：`sse_kind()`（细粒度事件名字符串，如 `reasoning_delta`/`subagent_child`/`plan_handoff`）、`sse_data()`（与 SSE 线格式一致的 JSON payload）、`coarse_kind()`（12 变体 `EventKind`，仅作 DB `type` 列与回退）。web 与 TUI 都走这三方法，live 广播与 replay 的 kind/payload 完全一致。
- `run_subagent` 子事件持久化改用 `cev.coarse_kind()` + `cev.sse_data()` + `Some(cev.sse_kind())`，删除旧的 `event_kind_from_str`（合并进 `coarse_kind`/`sse_kind`）。

### `crates/store/src/libsql_store/schema.rs`
- `SCHEMA_VERSION` 1 → 2。`bootstrap` 改为：CREATE 全表后读已存版本，仅当 `prev < SCHEMA_VERSION` 时跑 `migrate(prev)`。
- 新增 `migrate(from)`：v2 执行 `ALTER TABLE session_events ADD COLUMN sse_kind TEXT`（nullable，旧行仍合法）。新库（version None）已含全量 schema 故跳过迁移，仅写版本号——幂等且对存量库安全。

### `crates/store/src/libsql_store/events.rs` + `crates/store/src/types.rs`
- `SessionEventRecord` 新增 `sse_kind: Option<String>`（`#[serde(default)]`，向后兼容）。`append`/`after` 读写该列。

### `crates/web/src/handle.rs` + `crates/web/src/api.rs`
- `SseEvt::from_session_event` 从 ~90 行内联 match 精简为直接调 `ev.sse_kind()/sse_data()/coarse_kind()`；`drain_to_completion` 持久化时带上 `sse_kind`。
- `GET /events` 的 `get_events` replay 优先取 `r.sse_kind`，`None` 时回退 `event_kind_str(r.kind)`——旧行记录仍可回放（降级为粗粒度名）。

### `crates/tui/src/worker.rs`
- 新增 `persist_event(store, sid, sev)`：fire-and-forget `tokio::spawn` 把每条 SessionEvent（含 AgentSwitch/TranscriptReset/Compaction 等控制事件）写入 `session_events`（带 `sse_kind`），与 web drain 路径一致——**TUI 驱动的会话现可被 web/SSE 回放**。Prompt/SwitchAgent/SwitchAndStart/Compact 各派发点全部接入。

### TUI UX
### `crates/tui/src/key_handler.rs` + `crates/tui/src/app.rs` + `crates/tui/src/keybind.rs`
- 新增 `KeyAction::SwitchAgentNoClear`：输入框恰为 `t` 时按 Tab，切 act↔plan 但**不清空 transcript**（区别于 Shift+Tab 的 handoff）。`t`+Tab 对较长输入（如 `test`）不触发，走正常 submit。帮助文案同步。

### `crates/tui/src/render.rs` + `crates/tui/src/app_helpers.rs`
- 正文顶部边框（scroll_y>0 时）新增右对齐 `⬆`（U+2B06）置顶跳转指示，`MouseHits::top_btn` 导出点击命中 rect；点击即 `scroll=0` + 取消 follow。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| schema v2 迁移加 sse_kind（旧事件 None、新事件回写、幂等） | `schema_migration_v1_to_v2_adds_sse_kind` | `store/tests/store_integration.rs`（新增） |
| bootstrap 后版本=2 | `schema_migration_versioning` | `store/tests/store_integration.rs`（更新断言） |
| 全 16 变体 replay kind 与 live 一致 | `replay_kind_matches_live_kind_for_all_variants` | `web/tests/replay_fidelity.rs`（新增） |
| t+Tab act→plan 不清空 | `t_then_tab_in_act_mode_switches_without_clear` | `tui/src/app_tests.rs`（新增） |
| t+Tab plan→act 不清空 | `t_then_tab_in_plan_mode_switches_without_clear` | `tui/src/app_tests.rs`（新增） |
| 长输入不误触发和和弦 | `t_then_tab_does_not_fire_on_longer_input` | `tui/src/app_tests.rs`（新增） |
| 空输入 Tab 仍无操作 | `empty_input_tab_still_none` | `tui/src/app_tests.rs`（新增） |
| 下滑时顶部出现 ⬆ + 导出 top_btn | `body_top_arrow_when_scrolled_down` | `tui/src/render_tests.rs`（新增） |
| 顶部时无 ⬆、top_btn=None | `body_no_top_arrow_when_at_top` | `tui/src/render_tests.rs`（新增） |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 全绿，0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

提交前修复两处 gate 缺口（`tui/src/app_tests.rs`）：重复 `#[test]` 属性（clippy `duplicate-macro-attributes`）+ `shift_tab_toggles_plan_act` 误丢 `#[test]`（dead_code）。

## Impact Surface
- **web/SSE 回放**：历史与 TUI 驱动会话现可细粒度回放；旧行记录降级为粗粒度名（向后兼容，不报错）。
- **store**：schema v1→v2 自动迁移；新库/存量库幂等。`SessionEventRecord` 序列化新增可选 `sse_kind` 字段（旧 JSON 仍可反序列化）。
- **TUI**：新增 `t+Tab` 和弦与置顶 `⬆` 按钮；worker 副作用为写 store（无 store 时 no-op）。
- 不影响：`Store` trait 签名、CLI 命令、session 运行时语义（仅持久化字段补充）。

## Related Docs
- [agents/store](../../agents/store/index.md)
- [agents/session](../../agents/session/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/tui](../../agents/tui/index.md)
