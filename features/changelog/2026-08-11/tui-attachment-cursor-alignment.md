# 图片附件徽标行内占位导致光标错位

## Context

Composer 在挂载图片附件时会先渲染一行 📎 附件徽标（`render_composer` 内首个 inner
content line），文本从下一行开始。但绘制闭包的布局计算只按 `input_rows + 2` 推算
composer 高度，scroll/cursor 又只感知 `composer_inner_h`——三者均不知道徽标偷占了 1
个 inner line。后果：

- 光标恒比实际文本行高出 1 行（落在徽标行上方），越往后越明显；
- 末行被裁剪且不可滚动（`max_scroll` 把已被徽标消耗的那行也算进了可视高度）。

plan 模式下附件被显式过滤（`plan_mode.is_some() → &[]`），不受影响。

## Change Summary

- `composer::cursor_screen_position` 新增 `badge_h: u16` 参数，y 公式由
  `area_y + 1 + (row - scroll)` 改为 `area_y + 1 + badge_h + (row - scroll)`，
  使光标下移以对齐徽标之下的文本行。`badge_h=0` 时与旧行为逐字节一致。
- `render.rs` 绘制闭包新增 `badge_h` 计算，与调用点的 plan-mode 过滤严格一致：
  `!plan_active && !pending_images.is_empty() → 1`，否则 `0`。
- composer 块高 `composer_h` 加上 `badge_h`（块整体增高 1 行以容纳徽标）。
- 引入 `text_h = composer_inner_h - badge_h`（徽标偷占的 inner line 从可用文本高度
  扣除），`max_scroll` 与 `composer_scroll` 均改用 `text_h`，使滚动边界正确。
- `place_cursor` 调用透传 `badge_h`。

全部改动局限于 TUI 渲染层，无 trait/store/数据形状/CLI/HTTP/prompt 契约变化；
`badge_h=0`（无附件或 plan 模式）路径与改动前字节一致。

## Validation

- `cargo test --workspace` → `TOTAL passed=2345 failed=0` (全二进制汇总，0 failed)
- `cargo clippy --workspace --all-targets -- -D warnings` → Finished，零警告
- `cargo build --workspace` → Finished

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 附件徽标存在时光标落在文本行（纯函数） | `render::tests::cursor::place_cursor_with_badge` | `crates/tui/src/render_tests/cursor.rs` |
| 徽标渲染于首行、文本下移一行、光标与文本对齐（端到端绘制） | `render::tests::cursor::composer_badge_renders_and_cursor_aligns` | `crates/tui/src/render_tests/cursor.rs` |

## Related Docs

- 附件徽标渲染逻辑见 `agents/tui/index.md`（composer / wrap_rows 段）。
