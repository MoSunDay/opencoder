# Message.display 字段——用户原样输入成为所有回显面的单一事实源

Commit: c3ec226

## 背景

用户提交的 skill 原样输入（含 `$name` token）在多处 `User:` 回显面上被丢成
解析产物或 clean 文本：

1. TUI idle compound 提交（`app_submit.rs`）echo 取 `consumed_echo_text(&clean)`，
   clean 已剥 `$token` —— 回显丢 token；
2. 落库 user message 只有 clean text —— resume replay / SPA 快照 /
   `session show` 全部从 messages 取值，结构性显示 clean；
3. TUI 纯 skill 提交发送解析产物 `skill_trigger(name)` 文本，被 direct 路径以
   `synthetic=false` 落库 —— resume 后回显 "The `x` skill is now active..."；
4. SPA `turnsFromMessages` 不区分 `synthetic` —— `SKILL_TRIGGER` 渲染成 User
   气泡（与 TUI replay 跳过契约不一致）；
5. `session show` 文本视图显示 `m.text()` = clean —— 非 verbatim。

## 核心方案

给 `Message` 增加显示专用 `display: Option<String>` 字段作为「回显面单一事实
源」：落库时写入用户原样输入，所有回显面（TUI echo/replay、SPA、
`session show`）优先读它；LLM wire 与 context 构造只读 `blocks`
（`lower_messages` 不触碰新字段），天然隔离。解析逻辑（token 剥离、
`SKILL_TRIGGER` 注入、`deliver_body_once` payload）保持现状全部后置于消费时。

## 变更

- **`crates/core/src/message.rs`**：`Message.display` 字段
  （`#[serde(default, skip_serializing_if = "Option::is_none")]`，旧 JSON/旧行
  反序列化为 `None`）；新增 `Message::user_with_display(id, text, display,
  images)` 纯函数构造器。单测：旧 JSON 兼容、roundtrip、None 不序列化。
- **`crates/store`**：schema v13→v14，`messages.display` TEXT 列
  （CREATE TABLE + `add_column_if_absent` 迁移，存量行 NULL→回退 blocks）；
  `INSERT_MESSAGE`/`load`/`load_after` 全链路读写列。
  迁移测试：v13→v14 增列、存量行读 `None`、新行 round-trip；
  全部版本断言 13→14。
- **`crates/session`**：
  - `record_compound`（queue/steer 消费路径）：record 的 user message 与纯
    skill 注入的 synthetic `SKILL_TRIGGER` 均带 `display = Some(原样 rest，含
    $token)`；
  - direct 路径（`runner/mod.rs::run`）：resolve 前捕获 `raw_user_text`，
    record 带 display；text 被剥空时 verbatim 经 `entry_drain_mode` 传给注入
    的 trigger（`trigger_display` 参数）—— 与 `record_compound` 注入语义对
    齐，且不会双重注入（`entry_drain_mode` 仍是 direct 路径唯一注入点）。
  - 集成测试 `skill_display_verbatim.rs`：`$review fix the bug` 直发/队列两路
    → text clean、display 原样、token 不进任何 message text、不进
    MockChatClient 请求；`$review` 纯 skill → synthetic trigger + display
    verbatim；display 列经 store round-trip。
- **`crates/tui`**：
  - `app_submit.rs`：idle echo 改 `consumed_echo_text(&text)`（原样，compound
    尾参保留 `$token`）；纯 skill 提交改发原样 `$name`（删除
    `skill_trigger` 特例，runner 侧激活幂等），`skill_display.rs` 的
    `skill_trigger` 及其测试删除；
  - `session_ui/replay.rs`：`synthetic && display.is_none()` 才跳过；渲染文本
    优先 `display`（legacy 行回退 blocks）。
  - 单测：skill trigger display 渲染/裸 synthetic 跳过、user turn display
    优先。
- **`crates/web`**：SPA `reduce.js::turnsFromMessages` 对齐 TUI replay 契约
  （`synthetic && !display` 跳过；user 文本 turn 优先 `m.display`）；vitest 新
  增 display/synthetic/legacy-fallback 用例；SPA dist 重新构建（include_bytes!
  内嵌）。`session show --json` 经 serde 自动携带新字段，无需改动。
- **`crates/cli`**：`session show` 文本视图抽 `show_message_line`（display 优
  先，legacy 回退 `text()`）+ 单测。
- **顺带收尾（预先存在的 in-flight 改动）**：TUI 引入 `unicode-width` crate 后
  4 个宽度期望测试未同步——`composer_tests`（U+2702/U+2934/U+2B05 实测
  1 列）、`app_display::TOP_ARROW_W` 10→9（⬆ 实测 1 列）、
  `render_tests/queue_panel` 命中矩形断言对齐 `glyph_hit_rect`
  `[glyph_x-1, glyph_x]` 语义。均按 0.2.0 oracle 实测值修正。

## 有意保留

TUI idle 路径的 `resolve_persist` 仍在提交时解析——idle 提交即消费边界（立即
开 turn），语义上已属"后面处理"；其本地 echo 已改原样，落库 display 为 clean
属已知边界（统一后置到 runner 需 `skill_handle` 共享 Arc 架构调整，留作独立
后续任务）。queue/steer 消费路径无此限制，display 全程 verbatim。

## 回归

- `cargo test --workspace` 全量（含 v14 迁移、display round-trip、echo 契约
  新测试）→ 全绿（结果见下）。
- `cargo test -p opencoder-session --test skill_display_verbatim`（3 passed）、
  `cargo test -p opencoder-store --test store_migrations`（10 passed）、
  `cargo test -p opencoder-store --test display_text`（5 passed）、
  `cargo test -p opencoder-store --test schema_bootstrap`（6 passed）、
  `cargo test -p opencoder-cli --lib`（87 passed）、
  `cargo test -p opencoder-tui --lib`（1595+ passed，含 session_ui replay
  display 契约与宽度修正）。
- SPA：`npm test`（13 文件 105 tests 全绿）+ `npm run build` dist 重建。
