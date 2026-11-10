# Say 流式一开始即退场整个 Step 阶梯（与 live 区同时清空）

一句话：StepGroup 下邻块一旦是 open Say（`done == false`），阶梯（折叠
计数行 `▸ N Steps` 与展开 ladder 行）整体停止渲染——与 turn live 区同一
时刻清空，流式期间转录只剩 `❯ Say:` 头行（running 提示）+ raw body；
seal（`done == true`）后按原口径合并为 `Say(n steps)` 头行，步数随合并
回归。`open` 等披露标记不动（纯渲染期退场），seal 后按原标记恢复。

## 行为口径

- 退场门控：`chat_flatten.rs` StepGroup 分支检测下邻块是否为
  `Assistant { done: false }`，命中即跳过 `flatten_step_group`（阶梯
  零行）；live 区仍按 `progress_active` 原门控输出（Say 块落地即解除
  progress——含空首 chunk，live 区与阶梯同帧清空，见下条）。
- 空首 chunk 同帧退场：`chat_stream.rs::append_text_delta` 对任意
  chunk（含空首 chunk）先解除 progress——Say 块落地即推进 turn floor
  越过阶梯，此后任何 walk（含 Done/Error）都无法再触达旧 group；不先
  解除会令 live 区滞留在流式 Say 旁、甚至 seal 之后。
- 行数/点击镜像：`chat_headers.rs::collect_headers` 同门控跳过阶梯行与
  点击目标（仅保留 live 区计数）；`chat_tests/line_accounting.rs` 镜像
  算法同步。
- seal 合并不变：`Say(n steps): <preview>` 头行、正文预览去重、open
  ladder 展开行照旧（`chat_step_render.rs`）。
- copy 模式同口径：`flatten_for_copy` 复用同一门控，流式期间复制文本
  不含阶梯行。

## 功能 → 测试名

- 阶梯随 Say 开始退场、seal 合并回归：
  render_tests::live::streaming_say_renders_standalone_in_the_transcript_until_sealed
- running 提示与阶梯行一起退役、Say 头行领跑：
  chat_tests::tool_collapse::running_hint_persists_until_say_begins
- 流式期间无计数行、seal 落地：
  chat_tests::step_group::live_preview::say_streams_in_the_live_region_and_lands_in_the_transcript_on_seal
- 行数镜像对齐：chat_tests::line_accounting（assert_line_accounting_matches 全部用例）
- 空首 chunk 同帧清空（防 live 区滞留）：render_tests::live::empty_first_say_chunk_still_clears_the_pre_say_surfaces
