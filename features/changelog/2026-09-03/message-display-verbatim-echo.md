# Message.display verbatim 回显契约

Commit: af71944

## 背景

用户原样输入（含 `$skill` token）在 5 个回显面被丢成解析产物：TUI idle echo 只显示 clean text、queue/steer 落库后 resume 无处取 verbatim、纯 skill 提交回显解析产物 trigger 正文、SPA 快照把 synthetic trigger 渲染成 User 气泡、`session show` 只打 clean text。skill 解析产物只应进 LLM context，不应进回显。

## 变更

- **core**（`message.rs`）：`Message` 新增 `display: Option<String>`（serde default + skip_serializing_if）；`user_with_display()` 构造器。`display` 是**回显侧单一真源**，永不进 LLM wire（lowering/估算/压缩只读 blocks）。
- **store**（`schema.rs` + `messages.rs`）：v13→v14 迁移加 `messages.display` TEXT 列（`add_column_if_absent` 守卫、可空、回滚安全）；INSERT/load/load_after 全链路读写（index 8）。
- **session**：`skill_resolve::record_compound` 两条路径落 verbatim display（mention 展开后原文）；直发路径在 resolve 前捕获 `raw_user_text`，verbatim 非空才落；`drain::entry_drain_mode` 新参 `trigger_display`——纯 skill 提交注入的 SKILL_TRIGGER（synthetic）携带 verbatim，resume 后 replay 显示 `$name` 而非 trigger 正文。双重注入结构性排除（drain_mode 时 trigger 不注入，队列优先契约保持）。
- **tui**：`app_submit.rs` idle echo 改原样（`consumed_echo_text`），纯 skill 提交直发原样 `$name`；`session_ui/replay.rs` 渲染优先 display，`synthetic && display.is_none()` 才跳过。
- **web SPA**：`reduce.js::turnsFromMessages` 对齐 TUI 契约（synthetic 无 display 跳过 + display 优先）；dist 已重建。
- **cli**：`session show` 文本视图 display 优先，旧行回退 `text()`；`--json` 自动携带字段。

## 已知边界（不阻塞）

- 节点中继 `DialogMessage`（node_protocol.rs）只透传 seq/role/blocks/created_at——经节点 fetch_messages 切片到 SPA 的消息无 display，回退 clean 显示（与改前持平，非回归）；随节点链路迭代补透传。
- queue/steer 与 direct 两路径 display 微差（mention 展开后 vs 展开前），`$token` 均保留，可接受。

## 测试清单

| 场景 | 用例 | 位置 |
|---|---|---|
| 直发 prompt 落 verbatim display + clean blocks | `direct_prompt_records_verbatim_display_and_clean_text` | crates/session/tests/skill_display_verbatim.rs |
| 纯 skill 提交 trigger display 为 verbatim token | `direct_pure_skill_trigger_display_is_verbatim_token` | crates/session/tests/skill_display_verbatim.rs |
| queued compound 落 verbatim display | `queued_compound_records_verbatim_display` | crates/session/tests/skill_display_verbatim.rs |
| display 列落库/读回 round-trip | `crates/store/tests/display_text.rs` | store 集成 |
| schema v14 迁移幂等（版本断言 14） | `crates/store/tests/schema_bootstrap.rs`、`tests/store_migrations.rs` | store 集成 |
| core 消息构造/serde 兼容 | `crates/core/tests/message_image.rs`、`tests/skill_contract.rs` | core 单测 |
| TUI replay：带 display 的 trigger 渲染、裸 synthetic 跳过 | `replay_renders_skill_trigger_display_and_skips_bare_synthetic` | crates/tui/src/session_ui.rs |
| CLI 文本视图 display 优先、旧行回退 | `show_message_line_prefers_display_then_blocks` | crates/cli/src/session_cmd.rs |
| SPA：display 覆盖 blocks / synthetic 无 display 跳过 / 旧行回退 | `renders user display text verbatim over recorded blocks` 等 3 例 | crates/web/spa/src/reduce.test.js |
| Rust 全量回归 | `cargo test --workspace`：253 个测试二进制全绿（3952 passed / 0 failed） | workspace |
| SPA 全量回归 | vitest 13 文件 110/110 通过；dist 重建同步 | crates/web/spa |
