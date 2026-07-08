Commit: (working-tree, pre-initial-commit)

# 滚动条短内容对齐修复 — 手动 thumb 替换 ratatui ScrollbarState

## 背景
消息少（内容 ≈ viewport 高度）时滚动条 thumb 不在底部，停在轨道中段。用户反馈「滚动条在消息少的情况下还没对齐」。

## 根因
ratatui `ScrollbarState` 的 thumb 公式使用 `max_viewport_position = content_length - 1 + viewport_length`。当内容 ≈ viewport 时分母被 `viewport - 1` 项膨胀，thumb 停在 ~52% 而非 100%。例如 21 行内容 / 20 行 viewport / 跟随到底 → thumb 在轨道 52% 处。

次要问题：
- 滚动条画在边框列上（`sb_area.x = inner.right()`）而非内部最后一列。
- `text_w = inner.width - 1` 始终预留滚动条列，即使不渲染滚动条时也留空列。
- 滚轮处理器用 `r.width - 2`（全 inner 宽度）计算换行，与 render 的 `inner.width - 1` 不一致。

## 变更
- **`crates/tui/src/render.rs`**：移除 ratatui `Scrollbar`/`ScrollbarState`/`ScrollbarOrientation` 依赖，替换为手动 `draw_scrollbar()` 函数——用简单比例 `scroll_y * max_off / max_scroll` 定位 thumb，无 `-1+viewport` 失真。thumb 用 `▒`（medium shade），轨道用 `│`。位置移至 `inner.right() - 1`（边框内最后一列）。
- **`crates/tui/src/app.rs`**：滚轮处理器的换行宽度从 `r.width - 2` 改为 `r.width - 3`，与 render 的 `text_w = inner.width - 1` 对齐。

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 260 passed | 261 passed |
