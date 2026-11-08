# live 并入转录滚动窗共一条滚动条；流式 Say 期间阶梯保持收起

一句话：两条收口，部分取代
live-region-steps-only-say-in-transcript.md（其中 **Say 入转录、共享
滚动**保留；被取代的是实时区**几何**与 Say **合并时序**）。(1)
**一条滚动条**：turn live region 不再是转录框内的独立底部条
（`strip_rect` / `render_strip` 与 `content_h` 预留分支删除），而是并入
转录滚动窗口的**尾部** —— `render_body` 把转录可见切片
（`ViewportCache::visible_window`）与 live 切片（`slice_window`）拼进
**同一个** wrapping Paragraph（`scroll((top_skip, 0))` 吃掉部分首行），
滚动条 / follow / clamp 全按合账 `total_all = 转录行 + live 换行行`：
live **永不截断**，装不下就出滚动条，上滚可见 live 开头。(2) **流式
期间阶梯保持收起**：open Say（`done == false`）在转录中以**独立**形态
流式 —— `say:` 头行 + running spinner + 完整 raw body（缩进 4、无预览
去重）；邻接阶梯保持普通折叠 `▸ N Steps` 行，`❯ Say(n steps)` 合并头行
**只在 seal 后**成立（sealed 走合并 + 渲染正文 + 头行预览去重的原
逻辑）。事件状态机零改动；SPA 本轮零改动（滚动条本就统一、流式期
阶梯本就折叠）。

## 行为口径

- 一窗拼装：`live_rows = wrapped_total(live_lines, text_w)`（按
  `render_viewport::wrapped_rows` 计 wrapping Paragraph 实际渲染行数），
  `total_all = total_rows + live_rows`；窗口 = 内容切片
  `visible_window(scroll_y, content_vis)` + live 切片
  `slice_window(live_lines, text_w, scroll_y.saturating_sub(total_rows),
  可见高 - content_vis)`；`scroll_y >= total_rows` 时内容切空、live 的
  部分首行由 paragraph `top_skip` 吃掉（与内容侧同机制）。
- hit-rect 只记**可见内容行**（`content_vis = min(可见高, total_rows -
  scroll_y)`）—— live 尾巴无点击目标；live 仍 render-only，永不进
  `flatten_with`（选择 / 行数统计不可见）。
- 流式 Say 独立渲染：`say:` 头行 + ` ⠋ running`（warn 色）+ raw body
  全量（`assistant_rows(raw)` 缩进 4）；seal 后才与邻接阶梯合并为
  `Say(n steps)` 头行（`N Steps` 行消失、正文换渲染版并按
  `merged_say_body_decision(rendered)` 去重）。`SayHeader.streaming`
  字段与合并头行上的 spinner 删除，
  `say_preview_for` / `merged_say_body_decision` 收敛为只吃 `rendered`。
- live 清空时机不变：Say 开始（`progress_active` 解除）或 turn 结束
  （Done/Error）即刻清空。
- `chat_headers.rs` 镜像同步：open Say 独立头行 +1 + raw 行数，sealed
  才按合并对计 —— 行数统计与渲染两端口径一致。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 一条滚动条覆盖内容 + live 尾（滚动条按 total_all 计，新增） | `unified_scrollbar_covers_content_and_live_tail` |
| 巨大 live 骑滚动窗尾部：保尾、live 开头可上滚到达（重写） | `huge_live_rides_the_scroll_window_tail_kept_and_start_reachable` |
| 流式 Say 独立入转录直到 seal（`say:` 头行 + spinner + raw body；seal 后合并，语义重述） | `streaming_say_renders_standalone_in_the_transcript_until_sealed` |
| live 区内容与排布（当前 step thinking + 阶梯同款调用行、贴转录尾、`[tok cost]` 之上） | `live_strip_sits_below_say_rows_and_above_tok_cost_border` |
| running 提示持续到 Say 开始 | `running_hint_persists_until_say_begins` |
| open Say 可整段复制、无实时区 chrome | `streaming_say_is_copyable_transcript_content_without_chrome` |
| Say 组行边界尾随空行恰一条（流式期即成立） | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |
| 行数统计镜像（chat_headers 与 flatten 口径一致） | `chat_tests/line_accounting.rs`、`chat_tests/step_group*.rs` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` → 1813 passed / 0 failed
  （1810 + `bash_exec` 5：首次经 `systemd-run` 跑因缺 `HOME` 使登录
  shell 报 `/.cargo/env` 噪声，补 `--setenv=HOME=/root` 后全绿，与
  本轮改动无关）；clippy `--all-targets` 0 warning；
  `cargo fmt -p opencoder-tui --check` clean。
- 事件状态机（`opencoder-session` 等）零改动，不涉及其它 crate 门禁。
