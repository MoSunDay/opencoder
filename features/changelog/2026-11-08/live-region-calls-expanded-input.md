# 实时区 function call 展开展示：名称 + 完整输入，结果收起（TUI + SPA）

> **已取代**：本条的「名称行 + 完整输入展开」形态已被
> `live-region-calls-match-ladder.md` 取代 —— 实时区调用行改为与阶梯
> step 完全一致的共享渲染（聚合行 + 单行头）；「结果收起」口径保留
> （TUI `show_results=false` / SPA 默认关闭折叠）。

> **内容已扩展**：实时区此后还渲染**流式 Say**（`💬 Say` 标记 +
> 全文；Say seal 前不进转录），见 `live-region-hugs-content-say-deferred.md`。

一句话：实时区（thinking 下方）的 function call 行从「折叠单行」改为
**展开**形态 —— 每个调用渲染为 `❯ name` 名称行（运行中带 spinner /
running tag）+ 其**完整输入**（缩进 8，muted；字符串原样、其余 JSON
pretty —— 与 SPA `fmtValue` 同口径，TUI 新增 `ToolCall.name/input` 字段
与 `chat_helpers::fmt_input`）；调用**结果一律收起**：TUI 实时区永不
渲染 `output`（`expanded` 在实时区被忽略，结果下钻仍归 transcript 阶
梯），SPA 实时区结果收在默认关闭的 `result` 折叠里（点击可开）。TUI
实时区不再有 `N Function calls` 聚合行（调用行直列在 thinking 之下）；
transcript 阶梯渲染**逐字节不变**（`push_step_details` 拆出共享的
`push_thinking`，聚合行/单行头/展开输出照旧）。

## 行为口径

- 实时区调用行 = `    ❯ name`（accent 粗体，运行中尾随 spinner）+
  完整输入展开（indent 8，muted）；无聚合行、无 step 计数行、无结果。
- 结果收起：TUI 实时区忽略 `expanded`/`output`（阶梯独占结果下钻）；
  SPA `LiveCallRow` 的 `result` 折叠默认关闭、非受控。
- 阶梯不变：L2 聚合行、L3 单行头（`▸ name 摘要`）、展开输出、
  `collapse_all_collapsible` 等全部照旧；`ToolCall` 新增字段仅服务实时区。
- 回放路径（`session_ui/replay.rs`）同样填充 `name/input`，历史会话
  实时语义一致。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 实时区调用行展开（名称 + pretty 输入行）、聚合行移出实时区 | `running_calls_stream_with_spinner_and_the_ladder_stays_folded` |
| 实时区结果永不渲染（阶梯展开后仍不渲染）、阶梯独占下钻 | `live_results_stay_collapsed_the_ladder_owns_drilldown` |
| 渲染条落位不变（Say 下方、`[tok cost]` 上方）且含展开输入 | `live_strip_sits_below_say_rows_and_above_tok_cost_border` |
| SPA 实时区 input 免点击可见 | `live call rows show the input expanded without clicking` |
| SPA 实时区 result 默认收起、点击展开 | `live call results stay collapsed until opened` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` → 1809 passed / 0 failed；
  `cargo clippy -p opencoder-tui --lib --tests` → 0 warning；
  `cargo fmt -p opencoder-tui --check` → clean。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx` → 28 passed / 0
  failed。
