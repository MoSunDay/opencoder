# 实时区 function call 渲染与阶梯 step 完全一致（TUI + SPA）

一句话：实时区的调用行不再自创形态 —— 与转录阶梯 step **共享同一渲
染**：TUI `chat_live_turn.rs` 直接调 `push_step_details(…, calls_open=
true, show_results=false)`（`    ▾ N Function call(s)` 聚合行 + 
`      ▸ name 摘要` 单行头、运行中 spinner），此前 7da665b7f 的
`❯ name` + pretty-JSON 输入块形态整体回退（`ToolCall.name/input` 字
段与 `fmt_input` 一并移除）；SPA `CallRows` 回退为直用 `ToolContent`
（与 L3 完全同款 `🔧 name · dur` 折叠行，input/output 都收在默认关闭
的折叠里）。**结果收起口径保持**：TUI 新增 `show_results=false` 分支
—— 实时区永不渲染调用输出（结果下钻仍归阶梯）；`💬 Say` 流式块、贴
合不截断、Say seal 才入转录等上一条口径全部不变。阶梯自身行为零变化。

## 行为口径

- 实时区调用行 = 阶梯 open step 的调用行逐字一致（聚合行 + 单行头
  + running spinner）；无独立形态、无输入展开块。
- `show_results`：阶梯路径恒 `true`（`c.expanded` 照旧）；实时区恒
  `false`（`expanded` 被忽略）。
- SPA 实时区调用行 = `ToolContent`（L3 同款）；input/result 均在默
  认关闭的折叠内，点开可见 —— 与阶梯交互一致。

## 功能 → 测试

| 功能 | 测试 |
| --- | --- |
| 实时区聚合行 + 单行头（阶梯同款）、running spinner、阶梯保持折叠 | `running_calls_stream_with_spinner_and_the_ladder_stays_folded` |
| 实时区结果仍永不渲染（阶梯展开后依旧） | `live_results_stay_collapsed_the_ladder_owns_drilldown` |
| 溢出占满框内区保尾（回归） | `huge_live_takes_the_whole_frame_tail_kept_instead_of_blanking` |
| SPA 实时区调用行与 L3 同款（折叠默认关闭） | stepsBlock.dom `live call rows render identical to the ladder L3 rows` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- TUI：`cargo test -p opencoder-tui --lib` → 1811 passed / 0 failed；
  clippy `--lib --tests` 0 warning（顺带修了两个既有的 useless-conversion
  警告）；`cargo fmt -p opencoder-tui --check` clean。
- SPA：`npx vitest run src/stepsBlock.dom.test.jsx` → 30 passed /
  0 failed（pendingSay/`💬 Say` 用例未动、全过）。
