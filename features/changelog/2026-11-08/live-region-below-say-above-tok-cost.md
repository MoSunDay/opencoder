# TUI 实时区移入 transcript 框内：Say 行下方、[tok cost] 边框上方

一句话：修正 TUI 实时区的落位 —— 实时区必须在 **Say 行的下方**，
此前它作为 transcript 外层的独立布局块渲染，落在了 `[tok cost]`
底部边框**之下**；现在它成为 transcript **框内的底部条**：`render.rs`
`render_body` 用 `render_live::split_inner` 把 `block.inner(area)` 切成
（内容区, 底部条），转录段落/滚动条/命中记录用内容区，底部条经
`render_live::render_strip`（尾部保留：空间不足时裁掉最旧行）渲染在
`[tok cost]` 边框**上方**、follow 指示之前。高度口径不变（`region` 的
`min(行数, 框高/3)` 上限）；内容口径不变（仅 `⏺ live` 标记 + 当前
step 的 thinking + call 行，无 step 计数行）。为守住
`render.rs` ≤800 行的工程上限，实时区布局/渲染助手抽到新文件
`crates/tui/src/render_live.rs`（`region`/`split_inner`/`render_strip`），
外层 Layout 不再含实时区 chunk。SPA 落位不在本条范围（其
`TurnLiveRegion` 仍为滚动区外固定面板）。

## 行为口径

- 实时区渲染在 transcript 框内部：转录内容（Say/阶梯/滚动条/命中
  记录）与实时条共享同一个 `block.inner(area)`，实时条固定贴在
  框底（`[tok cost]` 边框上方、follow 指示之前），Say 行始终在实时条
  上方。
- 尾部保留：实时条空间不足时从**最旧**行开始裁（保留 thinking/call
  等最新行，可能裁掉 `⏺ live` 标记行本身）；测试注释明示该语义。
- 高度上限、内容口径（thinking + call、无 step 计数）、settle 清空、
  flatten 永不产出 live 行等口径全部不变。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 实时条位于 transcript 框内：Say/阶梯行下方、`[tok cost]` 边框上方，flatten 复制文本不混入 live 行 | `live_strip_sits_below_say_rows_and_above_tok_cost_border` |
| 其余实时区口径（前移、open-call 回退、settle 清空、running tag 等）不变 | `live_preview::` 既有套件（`render_tests/live.rs` 之外，`chat_live_turn` 侧不受落位影响） |

定向回归（测试范围仅本次改动链路，未跑全量）：

- `cargo test -p opencoder-tui --lib` → 1808 passed / 0 failed（含新
  增 `live_strip_sits_below_say_rows_and_above_tok_cost_border` 及
  `render_tests` 全部调用点适配）。
- `cargo clippy -p opencoder-tui --lib --tests` → 0 warning；
  `cargo fmt -p opencoder-tui --check` → clean。
