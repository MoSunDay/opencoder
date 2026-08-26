Commit: (working-tree)

# TUI 附件徽章多行化 + 可点击 ✕ 删除

## 背景

composer 的 pending-image 附件指示器此前只渲染**一行**：单图显示文件名，多图折叠为 `{name} ×N`，且无任何删除入口——粘贴多张图后用户既看不全清单也无法移除误贴的单张，只能清空重来。本轮把徽章改为每图一行、行尾内联一个可点击的 `✕`（U+2715）删除按钮。

## 变更

### 新模块：附件徽章渲染（crates/tui）

- **`crates/tui/src/attach_badge.rs`**（新，186 行）：纯函数 `render_attach_badge(f, inner, pending_images, hits) -> u16`——每张 pending 图渲染一行 `📎 {filename}`（warn 色），标签按剩余宽度截断但**始终保住最后一个 cell** 给右对齐 `✕`；每个 `✕` 以 `AttachDelBtn { index, rect }` 注册进 `hits.attach_del_btns`；超过 `inner.height` 的行不渲染不注册；返回实际占用行数（空列表 → 0）。`index` 是渲染时刻的 `pending_images` 下标：rect 每帧重建、单击只删一张，故该下标仅当帧有效。
- **`crates/tui/src/lib.rs`**：注册 `pub mod attach_badge;`。

### 渲染接线与 composer 让位

- **`crates/tui/src/render.rs`**：
  - `MouseHits` 增加 `attach_del_btns: Vec<AttachDelBtn>`，每帧随其它按钮组一并 `.clear()`（~render.rs:51/:166）。
  - 删除 `render_composer` 内旧的单行 `{name} ×N` 内联 Paragraph 与"固定下移 1 行"逻辑，改为调用 `render_attach_badge` 并按其返回的实际行数下移输入区（render_composer ~:693）；plan-mode 过滤语义不变（badge_h 计算继续镜像该过滤器）。

### 点击删除（app_helpers）

- **`crates/tui/src/app_helpers.rs`**：`handle_mouse` 签名新增 `pending_images: &mut Vec<(String, String)>`；在队列按钮之后新增 attach ✕ 命中块——首个命中即 `pending_images.remove(btn.index)` 并消费事件（越界防御：`index < len()` 才删），优先级位于 thinking 头切换之前（app_helpers.rs ~:570/:625）。
- **`crates/tui/src/app.rs`**：唯一调用点透传 `&mut pending_images`（app.rs ~:700）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 每图一行 + ✕ rect/index 注册 | `badge_rows_and_del_buttons` | crates/tui/src/attach_badge.rs |
| 长文件名截断但保住 ✕ cell | `filename_truncated_keeping_del_cell` | crates/tui/src/attach_badge.rs |
| 行数以 inner.height 封顶 | `rows_capped_at_inner_height` | crates/tui/src/attach_badge.rs |
| 空列表零渲染零注册 | `empty_images_renders_nothing` | crates/tui/src/attach_badge.rs |
| 多徽章输入区下移 N 行 + ✕ 逐行 rect 断言 | `composer_multi_badge_rows_shift_input` | crates/tui/src/render_tests/cursor.rs |
| 点 ✕ 只删被点那张 | `attach_del_click_removes_only_clicked_image` | crates/tui/src/app_helpers_tests/mouse_tests.rs |
| handle_mouse 新参数适配 | 既有 mouse/wheel/scroll/arrow 测试补 `&mut vec![]` 实参 | app_helpers_tests/{mouse,mouse_wheel,mouse_scroll}_tests.rs、render_tests/arrow_click.rs |

- 全量回归：`cargo test --workspace` → 220 个测试目标全部 ok / 0 failed（提交前新鲜重跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → exit 0，零警告
- build：`cargo build --workspace` → exit 0
- 行数：attach_badge.rs 186 ≤ 400（新文件）；render.rs 784 ≤ 800、app_helpers.rs 766 ≤ 800、mouse_tests.rs 451 ≤ 800（迭代中）

## Impact Surface

- TUI 用户可感知：多图粘贴时每张一行完整可见；点击任一行行尾 `✕` 即删该张 pending 图；composer 输入区随徽章实际行数整体下移。
- 不影响：键盘提交流程、plan-mode 过滤语义、copy-mode 无装饰视图、web/session/store/CLI 边界零改动（`handle_mouse` 仅追加末位参数，外部无其他调用方）。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](../2026-07-26/image-pipeline-multimodal-fix.md)
