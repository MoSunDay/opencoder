# 实时区紧贴转录内容且不截断；流式 Say 只进实时区，落定才进转录（TUI + SPA）

一句话：两条口径。(1) **贴合 + 不截断**：实时条不再钉在 transcript
框底（短内容时与 Say 行之间留空隙）、不再有 框高/3 的高度上限与尾部
裁剪策略 —— TUI `render_live::strip_rect` 把条放在转录**已渲染内容的
下一行**（`y = min(总行数, 内容区高)`），高度 = 实时行数（仅当内容 +
实时条超出框体时压缩内容区、物理溢出时保留最新行）；框内、Say 下方、
`[tok cost]` 边框上方的位置不变。(2) **流式 Say 只进实时区**：TUI 的
open Say（`Assistant { done: false }`）不再是转录内容 ——
`chat_flatten.rs`/`chat_headers.rs`/copy 全部跳过 open Say（邻接合并
`Say(n steps)` 头行只在 `done: true` 后成立），实时区在 thinking +
调用行之后渲染 `💬 Say` 标记 + 流式全文；SPA 对称：`text_delta` 只
累积 `state.pendingSay`（不动 turns），`TurnLiveRegion` 新 `say` prop
渲染 `💬 Say` 块。Say 落地时机 = seal/flush 边界（LLM round 结束、
下一 tool/think 活动、terminal settle、新输入；attempt reset 丢弃），
落地即完整的合并 `Say(n steps)` 头行 —— 转录不再有 Say 的流式展示，
streaming 全部交给实时区。**TUI 事件状态机零改动**（仅渲染/flatten
侧），阶梯计数样式（含 settled 合并头行）不变。

## 行为口径

- 实时条贴合：短转录时条紧跟最后一行内容（无空行间隙）；满转录/
  滚动时条在内容区底（= 框内底部）；高度不设上限，物理溢出才保尾。
- 溢出回归（修复）：live 行数 ≥ 框内高时，条**占满整个框内区**（保
  尾、最新行可见），绝不因 `content_h == 0` 早退把 body 渲染成空白
  框 —— 转录随 live 回落到框高以下自动回归。
- 流式 Say 三阶段（TUI `say_streams_in_the_live_region_and_lands_in_
  the_transcript_on_seal`）：流式中 = 实时区 `💬 Say` + 全文、转录无
  Say 行且阶梯独立组行；seal = Say 落地成合并头行、实时区清空；
  Say 后新活动 = 下一轮 ladder 照旧。
- flush 边界两侧一致：tool_start/reasoning_delta/round_end/terminal/
  新输入触发落地（SPA `flushPendingSay`；TUI 既有 finalize 点），
  attempt reset 丢弃 pendingSay / open Say。
- `sayStreaming`/running 骑 Say 头行的展示口径退役（Say 落地即完
  整）；阶梯 running 提示在 Say 首块退役的既有口径不变。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 条贴合最后一行内容（上方无空隙）、输入行不截断、Say 下方/tok 上方排序不变 | `live_strip_sits_below_say_rows_and_above_tok_cost_border` |
| live 超过框高时条占满框内区保尾（不空白 body） | `huge_live_takes_the_whole_frame_tail_kept_instead_of_blanking` |
| Say 流式在实时区、seal 才落地合并头行（三阶段） | `say_streams_in_the_live_region_and_lands_in_the_transcript_on_seal` |
| running 标记只出现在实时区（Say 流式期间转录无 say 行） | `running_call_header_lives_only_in_the_live_region_and_never_leaks_into_copy_text`（含 live_text 扩展） |
| SPA text_delta 只累积 pendingSay、turns 不动 | `reduce.test.js` watermark/delta 组 |
| SPA flush 边界顺序（say 先于新 ladder 落地）、attempt reset 丢弃 | `reduce.order.test.js`、`reduce.test.js` |
| SPA 实时区渲染 pending Say（无 running steps 时也显示） | stepsBlock.dom `live region streams the pending Say under the 💬 Say marker` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` → 1810 passed / 0 failed；
  clippy `--lib --tests` 0 warning；`cargo fmt -p opencoder-tui --check`
  clean。
- SPA：reduce*.test.js（46/3/5/2）+ steps/reducer.test.js（31）+
  stepsBlock.dom.test.jsx（30）+ saypairs.e2e（4）+ subagentBlock.dom
  （7）及受影响链路 chat.dom 等 → 10 文件 145 passed / 0 failed。
