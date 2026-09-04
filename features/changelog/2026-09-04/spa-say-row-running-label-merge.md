Commit: af71944

# SPA Say 头行 running 保留 + Say 标签合并步数（`❯ Say(N step{s}): 预览`）

## Context

`1 Turn = n Steps + Say` 契约（`features/changelog/2026-09-03/say-pairs-steps-all-surfaces.md`）下，SPA 旧行为：Say 一出现 running Tag 就消失（`reduce.js` text_delta 先 `settleTurnProgress` 再 `appendDelta`，`bubbleItems.js` 的 `hasSay→false` 覆盖把 progressActive 冻死为 false），用户在 Say 流式期间看不到"还在跑"的信号；且阶梯头行 `❯ N Step(s)` 与 Say 文本是两行，TUI 同步的对齐渲染模型要求 Say 出现后 running **转移**到 Say 头行，头行同时合并步数。

## Change Summary

- **`spa/src/steps/reducer.js`**：steps turn 新增 `sayStreaming` 生命周期（全部 copy-on-write，不改输入数组）：
  - 新导出 `markSayStreaming(turns)`：与 `settleTurnProgress` 同一目标（`turnStepsIndex` = floor 起第一条 steps turn，即即将被 Say 关闭的阶梯）置 `sayStreaming: true`，已为 true 则原样返回；
  - 新导出 `clearSayStreaming(turns)`：清除所有 steps turn 的 flag（只拷贝带 flag 的 turn，全部无 flag 返回原数组）；内部 `clearSayStreamingInPlace` 供 mutator 风格的 `appendStepCall` 用；
  - `appendThinkDelta` / `appendStepCall` 的新建 steps turn 分支（Say 之下开新阶梯）先清所有已存在 turn 的 flag——同子回合内（尚无 Say）的追加走复用分支，永不触碰 flag。
- **`spa/src/reduce.js`**：text_delta 顺序锁定为 settle → **mark** → append（`reduce.order.test.js` 原顺序不变量保持）；新增内部 `settleTerminal(turns)` = `clearSayStreaming(settleTurnProgress(turns))`，替换 done/error/interrupted/transcript_reset/queue_consumed/steer_consumed/withUserTurn 全部收束点的 `settleTurnProgress` 调用，终态不留悬挂 running。
- **`spa/src/bubbleItems.js`**：assistantTurn content 新增 `sayActive = stepParts.some((p) => p.sayStreaming === true)`（恒 boolean；与 progressActive 汇总并列，hasSay→false 覆盖不动）。
- **`spa/src/stepsBlock.jsx`**：`StepsContent` 按是否有 Say 切头行——无 Say 维持 `❯ N Step(s)`（Tag 由 progressActive/openCall 回退驱动）；有 Say 改 `❯ Say(N step{s}): {首行预览}`（N=steps.length；单数 "1 step"；预览=say 文本 parts 拼接后第一非空行 trim，`image:true` 标记不算 Say 文本，空/纯空白 Say 冒号后留空），running Tag 由 `turn.sayActive === true` 驱动，error Tag（`!running && errored`）语义不变；Collapse 结构/keys/默认收起不变（不传 activeKey）。顶部注释更新：仅 Say 单行预览进入 label，正文仍在外面。
- **`spa/src/transcript.jsx`**：`AssistantTurnContent` 把完整 content（含 say、sayActive、progressActive）传入 `StepsContent`，Say 正文渲染不动。
- 回放/快照路径零迁移：`turnsFromMessages` 旧 turn 无 `sayStreaming`，恒 falsy 不显示 running。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| 首 Say 块置 flag、续块保持；settle 先于 mark 先于 append | `steps/reducer.test.js` (s1)（新增）、`reduce.order.test.js` (order-a)(ii-b)（新增断言） |
| Say 后新阶梯 reasoning/tool 清除旧 flag（appendThinkDelta/appendStepCall 新建分支） | `steps/reducer.test.js` (s2)(s2b)（新增） |
| done/error 清 flag | `steps/reducer.test.js` (s3)（新增） |
| 同回合无 Say 追加不碰 flag | `steps/reducer.test.js` (s4)（新增） |
| mark/clear 幂等、copy-on-write、目标为 floor 起首条 steps turn；无 flag 数组原样返回 | `steps/reducer.test.js` (s5)（新增） |
| 逐对 sayStreaming：该对流中 true、下一对激活时旧对 false、终态后全 false；progressActive 语义不变 | `saypairs.e2e.test.js` (a)（新增断言）、(c)（快照断言收敛为 `{...ladder1, sayStreaming: false}`，不放松） |
| sayActive 透传（有 flag true / 无 flag false）且 hasSay→false 的 progressActive 覆盖保持 | `bubbleItems.test.js`（新增 + 原用例补断言） |
| 有 Say 且 sayActive 缺省 → running 消失、label 为 `❯ Say(2 steps): …` 形态 | `stepsBlock.dom.test.jsx`（原 "12px gap" 用例拆分改写） |
| 有 Say 且 sayActive=true → Say 行出现 12px running Tag | `stepsBlock.dom.test.jsx`（新增） |
| Say 行 error Tag：!running && errored 照旧 | `stepsBlock.dom.test.jsx`（新增） |
| `N Steps + Say` 单气泡 + Say 行 label + 文档顺序 | `stepsBlock.dom.test.jsx`（更新 label 断言） |
| 纯 steps（无 Say）全部断言不变 | `stepsBlock.dom.test.jsx` 原用例保持 |

## 回归

- `cd crates/web/spa && npm test`：17 文件 **196 用例全绿**（基线 186 + 净增 10）。
- `bash scripts/build-spa.sh`：重建 `spa/dist`（index.html + static/app.js + static/app.css）。
- `bash scripts/check-spa-drift.sh`：no drift。
- 未触碰任何 Rust 文件与 `crates/tui`（并行任务的 TUI 未提交改动原样保留）。
