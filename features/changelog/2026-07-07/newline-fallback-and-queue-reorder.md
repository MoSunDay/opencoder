Commit: (working-tree, pre-initial-commit)

# 换行 fallback + follow-up 队列面板重排/删除

## 变更

### Part 1 — 多行换行 fallback（修 Shift+Enter 不识别）
- **根因**：`app.rs` 仅查 `KeyModifiers::SHIFT`，依赖 Kitty 键盘协议；非 Kitty 终端 Shift+Enter 落成裸 `\r`，且 `app.rs:628` 注释提到的 "Alt+Enter" 从未实现。这是终端协议物理限制，非代码 bug。
- **Enter 臂扩展**（`crates/tui/src/app.rs`）：条件由 `contains(SHIFT)` 改为 `intersects(SHIFT | ALT)` —— Alt+Enter 现在真正插入换行（几乎所有终端可靠）。
- **Ctrl+J 新增**（`crates/tui/src/app.rs` Ctrl-block）：Ctrl-block 原有 `_ => return KeyAction::None` 兜底会拦截一切 Ctrl 组合，故 Ctrl+J 必须落在 Ctrl-block 内部（`Char('j')` 臂）而非主 match。Ctrl+J = 字面 LF 字节，100% 跨终端可靠。
- **keybind.rs** 帮助文本更新为 `Shift+Enter / Alt+Enter / Ctrl+J   insert newline`。
- 保留 Shift+Enter 给 Kitty 协议终端。

### Part 2 — follow-up 队列面板：≤3、点击重排/删除（仅 queue 项）
- **身份追踪**：`queue_items: Vec<String>` → `Vec<(i64, String)>`，捕获 `admit_input` 返回的行 seq（原先丢弃）。steer 项保持只读 `Vec<String>`。
- **Store 重排原语**（`crates/store`）：新增 `Store::swap_input_order(session_id, seq_a, seq_b)` —— 事务内交换两行 `admitted_seq`，改变 drain 顺序（runner 仍按 `admitted_seq ASC`，无需改 runner）。`admitted_seq` 无 UNIQUE 约束，直接 `CASE WHEN` 交换即可。`delete_input` 已存在，本次接入 TUI。
- **渲染按钮**（`crates/tui/src/render.rs::render_queue_panel`）：黄色 `[queued]` 行尾追加 ` ▲ ▼ ✕` 控制字形（6 列），每个字形导出 1-cell hit rect 到 `MouseHits.queue_btns`。窄终端（avail_w ≤ 10）退化为无按钮。
- **鼠标路由**（`crates/tui/src/app.rs`）：`MouseEventKind::Down(Left)` 命中 ▲▼ → `store.swap_input_order` + 本地 swap；命中 ✕ → `store.delete_input` + 本地 retain。
- **新模块 `crates/tui/src/queue_panel.rs`（122 行）**：纯函数 `plan()` / `apply_swap()` + `QueueBtn`/`QueueBtnAction`/`QueueEffect` 类型。把可测试的重排/删除决策从 `app.rs`（已逼近 800 行上限）抽出。
- 显示上限 3 已存在（`render.rs` `max_lines = min(area.height, 3)`），本次复用。
- **Review 修复**：渲染时 head 不足 `cap` 会用空格补齐到 `cap`，使 `▲ ▼ ✕` 真正落在右边缘与 hit rect 对齐（否则短 prompt 下字形浮在文本后、点击错位）；抽出纯函数 `btn_x_offsets(width)` 统一几何并单测。
- **宽字符对齐修复**：队列行 head 原按 `chars().count()`（Unicode 标量）测宽，CJK/emoji 下 pad 过大、字形与 hit rect 错位且会换行；改用新增的 `composer::str_width`/`truncate_to_width`（按显示列截断 + 补 `…`），`▲ ▼ ✕` 与 `btn_x_offsets` 在任何字符宽度下都对齐。

## 涉及文件
- `crates/tui/src/queue_panel.rs` — **新增**，122 行（纯交互逻辑 + 8 单元测试）
- `crates/tui/src/lib.rs` — `pub mod queue_panel`
- `crates/tui/src/app.rs` — Enter/Ctrl+J 换行 + queue_items 类型 + seq 捕获 + 鼠标路由
- `crates/tui/src/render.rs` — `MouseHits.queue_btns` + `render_queue_panel` 按钮/hit-rect
- `crates/tui/src/keybind.rs` — 帮助文本
- `crates/tui/src/app_tests.rs` — 3 个换行测试
- `crates/tui/src/session_ui.rs` — queue_items 类型 + 测试数据
- `crates/store/src/store.rs` — trait `swap_input_order`
- `crates/store/src/libsql_store/inputs.rs` — `swap_input_order` 实现
- `crates/store/src/libsql_store/mod.rs` — Store impl 接线
- `crates/store/tests/inputs_integration.rs` — `swap_input_order_changes_drain_order`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Shift+Enter 插入换行 | `enter_with_shift_inserts_newline` | `tui/src/app_tests.rs` |
| Alt+Enter 插入换行 | `enter_with_alt_inserts_newline` | `tui/src/app_tests.rs` |
| Ctrl+J 插入换行 | `ctrl_j_inserts_newline` | `tui/src/app_tests.rs` |
| queue 上移交换前驱 | `up_swaps_with_predecessor` | `tui/src/queue_panel.rs` |
| queue 顶不上移 | `up_at_top_is_noop` | `tui/src/queue_panel.rs` |
| queue 下移交换后继 | `down_swaps_with_successor` | `tui/src/queue_panel.rs` |
| queue 底不下移 | `down_at_bottom_is_noop` | `tui/src/queue_panel.rs` |
| queue 删除返回 seq | `delete_returns_seq` | `tui/src/queue_panel.rs` |
| 本地 swap 重排 | `apply_swap_reorders_locally` | `tui/src/queue_panel.rs` |
| 按钮 hit-rect 几何（右边缘对齐） | `btn_x_offsets_pin_glyphs_to_right_edge` | `tui/src/queue_panel.rs` |
| swap 改变 drain 顺序 | `swap_input_order_changes_drain_order` | `store/tests/inputs_integration.rs` |
| 删除 pending 项保序 + 幂等 | `delete_input_removes_pending_and_preserves_order` | `store/tests/inputs_integration.rs` |

- 全量回归：`cargo test --workspace` → 258 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数 gate：新文件 `queue_panel.rs` 142 ≤ 400；`app.rs` 788 ≤ 800

## 执行方式
按文件归属并发：Wave 1 两个并发 `general` subagent（store crate ∥ tui 换行，文件零交叉），Wave 2 主 agent 单线做队列面板（4 文件紧耦合不可并行），Wave 3 回归 gate。
