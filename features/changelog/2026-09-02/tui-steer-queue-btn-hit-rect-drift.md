# steer/queue 面板 ✕/`>` 点击失效——宽度模型与渲染 oracle 统一 + 命中矩形加宽 + 子镜像 retain 补漏

Commit: 3adc115

## 背景

- **症状**：steer 行的 `✕`（删除）与 `>`（立即提交）有概率一起点不动；queue 行 `▲ ▼ ✕` 同理。
- **根因（两套宽度模型分歧，概率性来源）**：命中矩形钉死在右边缘固定列（`queue_panel.rs` 的 `btn_x_offsets`/`steer_btn_x_offsets`），而字形落点由 `composer::str_width` 的 pad 推出。该 pad 依赖手写的 `char_width` 近似模型，与 ratatui 0.29 实际布局所用 `unicode-width 0.2` 对同一字符分歧：
  - `U+2702..=U+27B0` 整段 dingbat 被记 2 列，但 ✕(U+2715)/✓(U+2713)/✂(U+2702) 实际渲染 1 列 → pad 偏大被高估，strip 被推向**左**，1×1 矩形全部落在空白上；
  - `U+FE00..=U+FE0F`（VS16）被记 0 列，但 ❤️ 等 emoji 呈现序列实际渲染 2 列（ZWJ 组合同理）→ strip 被推向**右**并溢出裁切。
  - 只要 steer/queue 文本含这类字符，该行可见字形整体偏离 1~N 列，两个 1×1 矩形成对点空——精确匹配「✕ 与 > 一起失效」的特征。
- **伴生缺陷（幽灵行）**：焦点在活跃 subagent 时面板显示子会话 `view.steer_items`（`app_display::steer_queue_sources`），但删除成功后只 retain 父镜像 `chat.steer_items` → 子行删除后仍挂在面板上，✕ 看似无效。

## 变更

- **统一宽度模型（根因修复）**：`crates/tui/Cargo.toml` 增加 `unicode-width = "0.2"`（与 ratatui 同版本，零新增编译产物）。`composer::char_width` → `UnicodeWidthChar::width`（控制符 `None` 记 0，钳制 0..=2）；`composer::str_width` → 序列感知的 `UnicodeWidthStr::width`（❤️/ZWJ 家族/keycap 与 ratatui 渲染一致）。`truncate_to_width`/`cursor_column` 随单点自动收敛；notepad 光标、keymap 截断、render 标签、copy_mode chip 等 8 处 `char_width` 消费方全部变准。
- **命中矩形加宽（容错兜底）**：`queue_panel.rs` 新增 `glyph_hit_rect`——steer 行 `✕`/`>` 与 queue 行 `▲ ▼ ✕` 的矩形由 1×1 改为 2×1（覆盖「分隔空格+字形」），相邻矩形互不重叠且不溢出 content_w（含滚动条溢出态）。残留 ±1 列 pad 漂移不再致死。
- **子镜像 retain 补漏**：`handle_mouse` Delete 成功分支复用 `collapse_view` 式聚焦辅助，对焦点视图的 `steer_items` 同步 retain（`handle_mouse` 随本改动从 `app_helpers.rs` 迁入新文件 `app_mouse.rs`，`app_helpers` 保持 re-export，调用点零变更；`app_helpers.rs` 由此回到 578 行 < 800 迭代上限）。幽灵行消除。

## 测试覆盖（rules/01/02/03）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 宽度 oracle 表测：dingbat/箭头渲染 1 列（✕ ✓ ✂ ❤ ⇱ ⬅ ⬆）、真宽字符 2 列、控制符 0 列 | `char_width_matches_unicode_width_oracle` | crates/tui/src/composer_tests.rs |
| str 级序列感知：`str_width("✕")==1` 回归钉 + ❤️/ZWJ 家族/旗帜/keycap/decomposed 合计与渲染一致 | `str_width_is_sequence_aware_like_ratatui` | crates/tui/src/composer_tests.rs |
| **渲染级对齐（核心回归）**：含 dingbat 文本的 steer 行，Delete/Submit 矩形格子恰为 `✕`/`>` 且左列为分隔空格 | `steer_btn_rects_contain_rendered_glyphs_dingbat_text` | crates/tui/src/render_tests/queue_panel.rs |
| 渲染级对齐：ZWJ 家族 emoji 文本的 queue 行 ▲ ▼ ✕ 三矩形 | `queue_btn_rects_contain_rendered_glyphs_emoji_text` | crates/tui/src/render_tests/queue_panel.rs |
| 渲染级对齐（滚动条溢出态）：strip 左移一列后矩形仍咬住字形、不侵入滚动条列 | `steer_btn_rects_stay_on_glyphs_with_scrollbar_overflow` | crates/tui/src/render_tests/queue_panel.rs |
| 2×1 矩形几何：span [glyph_x-1, glyph_x]、相邻矩形不重叠、右缘不出界 | `glyph_hit_rect_spans_separator_and_glyph` | crates/tui/src/queue_panel.rs |
| 矩形加宽行为：点字形左侧一格（分隔空格列）仍触发删除且落库 | `steer_delete_click_on_separator_space_still_hits` | crates/tui/src/app_helpers_tests/mouse_tests.rs |
| 子镜像删除：焦点活跃 subagent 点子行 ✕ → 子视图镜像同步移除、父镜像不受扰、store 落库 | `steer_delete_removes_focused_subagent_mirror_row` | crates/tui/src/app_helpers_tests/mouse_tests.rs |

**牙齿验证**：三个渲染级对齐测试在旧宽度模型（dingbat 记 2 列 + per-char 求和）下全部 FAILED（「glyph column does not hold the rendered glyph」），新模型下全绿——rect/字形未来任何漂移都会被立即卡住。既有测试零弱化：`StubStore` 仅将 `delete_input` 从 panic 改为成功并记录 seq（供删除路径断言），其余方法维持 panic 语义；旧 composer 钉测中与新模型一致的条目原样保留。

## 回归门（rules/02）

- `cargo test -p opencoder-tui`：lib 1602 passed + 全部集成套件 0 failed。
- 本修复相关的 10 个测试（composer 宽度 ×3、queue_panel 几何 ×1、渲染级对齐 ×3、鼠标行为 ×3）在当前工作树单独复跑全绿。
- `cargo test --workspace --no-fail-fast`：最终门 0 failed（隔离 worktree @ 3adc115 复验：249 test targets / 3914 passed / exit 0，含 doctest；并行任务 working-tree 中间态曾致 1918 阶段唯二确定性失败，均非本变更引入，见下）。
- `cargo clippy -p opencoder-tui --all-targets` 0 warning；`cargo fmt` 已套用。

## 既存失败修复（非本症状引入，为过回归门顺带收敛；均为 working-tree 阶段动作，schema 钉修正属并行 schema WIP，未随本 Commit 落库）

- `crates/store/tests/schema_v4_migration.rs`：工作树既存的 schema v13→v14 WIP 已把 `SCHEMA_VERSION` 提到 14 并同步了 `display_text.rs`，但漏改本文件两处「latest = 13」断言（报错自证：DB 为 14、断言期望 13）。按 WIP 自身方向机械修正 13→14，全量回归由此解阻。
- tui 侧 doctest 编译失败（`chat::ToolGroupState` 路径）属**另一并行任务的 ToolGroupState 迁移中间态**（chat.rs/replay.rs 在本次验证进行中仍在被持续改写），不属本修复半径，未代为收口，留待该任务自身的回归门处理。

## 范围外已记录（后续任务候选，不属本症状主链）

- `shift_held` 闩锁吞全鼠标事件（`app.rs` + `terminal.rs`）：tmux 不发 release 时全界面点击死。
- `>` 在 `running` 旗标过期时对已取消 token 空转（`steer_dispatch.rs` `_ => {}`）。
- 会话切回用快照恢复镜像可能带出已消费行（`app_task.rs`）。
