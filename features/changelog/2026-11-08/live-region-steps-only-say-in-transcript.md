# 实时区只渲染运行中 step；流式 Say 即时进转录（TUI + SPA）

一句话：两条口径，取代此前「流式 Say 只进实时区」的安排（见
live-region-hugs-content-say-deferred.md）。(1) **实时区瘦身**：turn
live region 只渲染**当前（最后一个）step** 的 thinking 文本 + function
call 行（与阶梯共享 `push_step_details`，calls open、结果永不渲染），
**无 step 计数行、无 Say** —— Say 一开始（`progress_active` 解除）区域
即刻清空。(2) **open Say（`Assistant { done: false }`）是转录内容**：
流式期间即渲染**最终形态** —— 邻接合并的 `❯ Say(n steps)` 组头行立即
成立（running 提示随流骑行其上）、下方流式 RAW body 行共享转录滚动，
seal 只去掉 running 提示，**零形状跳变**。配套修复实时条两个 bug：
转录溢出时条不可见（`strip_rect` 此前拿总行数定位、把条推出框外，改为
`min(总行数, 框内高)` 的可见行数）；长行换行后保尾错位（`render_strip`
此前按未换行的原始行数算 skip，改为 `wrapped_rows` 换行后行数求和再
scroll，保尾精确）。事件状态机零改动，仅渲染 / flatten / copy 侧。

## 行为口径

- 实时区 = `⬥ live` 头 + 当前 step 的 thinking + 调用行（阶梯同款，
  结果下钻仍归阶梯）；新 step 到来后旧 step 只活在折叠的阶梯里。
- Say 开始 → 组 `progress_active` 解除 → 实时区清空；open Say 在转录
  以最终形态流式：合并头行 + running 提示 + raw body（首行并入头行
  预览），落定仅去提示；`say:` 独立头行同口径。
- 贴合定位：`strip_rect(inner, content_rows.min(inner.height), live_len)`
  —— 溢出/滚动时条贴**可见**内容窗的下一行，不再被推出框。
- 换行保尾：`render_strip` 按 `render_viewport::wrapped_rows` 求和后的
  实际渲染行数计算 skip，物理溢出才裁剪。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| open Say 即时进转录（合并头行 + running 提示 + raw body；实时区无 Say、无 step、seal 仅去提示） | `say_streams_in_the_live_region_and_lands_in_the_transcript_on_seal`（语义更新）、`streaming_say_renders_in_the_strip_until_sealed`（语义更新） |
| 实时区只留当前 step（无计数行），新 step 折叠旧 step | `running_reasoning_streams_in_the_turn_live_area`、`a_new_step_moves_the_live_area_and_folds_the_previous_one` |
| running 提示随流骑 Say 头行、Say 开始前持续 | `running_hint_persists_until_say_begins` |
| 转录溢出时实时条仍可见（可见行定位回归，新增） | `live_strip_stays_visible_when_transcript_overflows` |
| 长行换行后保尾精确（wrapped 计数） | `huge_live_takes_the_whole_frame_tail_kept_instead_of_blanking` |
| open Say 可整段复制、无实时区 chrome | `streaming_say_is_copyable_transcript_content_without_chrome` |
| Say 组行边界的尾随空行恰一条（流式期即成立） | `tool_turn_boundary_keeps_exactly_one_trailing_blank` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` → 1812 passed / 0 failed；
  clippy `--all-targets` 0 warning；`cargo fmt -p opencoder-tui --check`
  clean。
- 旧签名调用方已随本链路迁移：`app_loop_tests/sidecar_display_tests.rs`、
  `remote/tests/mod.rs`、`remote/tests/protocol.rs` 的 open-Say 断言从
  live 区取证翻转为转录取证（flatten 直接含流式文本）。
- SPA 同口径（`stepsBlock.jsx` / `chat.jsx`）：`TurnLiveRegion` 去掉
  `say` prop 与 `💬 Say` 块（仅 thinking + 调用行），连同新的
  `StreamingSay`（pendingSay 原文、无标记）一起移入 transcript 滚动
  容器内、消息列表之下 —— 与消息共用滚动条；auto-stick 仅在贴近底部
  时跟随（手动上滚优先）。定向：stepsBlock.dom + chat.dom 44/44。
