Commit: (working-tree, 基于 4bafa6c：评审证据链闭合后的 TODO 跟进)

# 评审跟进：Say 配对收尾——顺序不变量锁 + 帧级 e2e + 瞬态失败溯源

## Context

[N Steps 与 Say 成对](say-pairs-steps-all-surfaces.md) 评审结论 ready，遗留 4 项 TODO：真机视觉验证、瞬态失败溯源、P3 注释/锁补强、仓库记忆索引。本篇为跟进执行。工作区与「SSE resync 去重」并发会话共享，其所属文件（`sse.js`/`sse.test.js`/`reduce.js`/`reduce.test.js`/`chat.jsx`/`chat.dom.test.jsx`/`dist/static/app.js`）一律只读不碰。

## Change Summary

- **`spa/src/steps/reducer.js`**：`turnHasSay` 注释收敛为结构性不变量声明——`turnFloor` 返回最后一个非空 Say（或 user 文本边界）之后的位置，故 floor 之上不可能存在 Say，该函数恒 false；保留仅作 progress gate 纵深防御，真正的冻结在 `settleTurnProgress`。零逻辑变更。
- **新增 `spa/src/reduce.order.test.js`（3 例）**：reduce 层锁「`text_delta` 首个非空 chunk 先 `settleTurnProgress` 再 append Say turn」顺序不变量（此前仅锁在 steps 层 (c2d)/(d2)）。位点：`reduce.js::foldFrame case 'text_delta'`（settle 严格先于 append）。(order-a) steps 先于 say 落地且 `progressActive` 已冻结；(order-b) 续写 chunk 只扩 say 不回触冻结梯；(order-c) Say 后 reasoning 开新梯、冻结梯保持单 step 单 call。
- **新增 `spa/src/saypairs.e2e.test.js`（4 例）**：帧级 e2e 锁三项视觉语义（真机浏览器验证保持人工项——本环境 chromium 在、playwright-core 未装，且未消耗真模型跑长流）：(a) 一 run 两回合 `[2 Steps + Say][1 Steps + Say]` 交替，双梯各自被自己的 Say 冻结、梯 2 严格在 Say 1 之下、done 后 `pendingEcho` 退役；(b) `transcript_reset` 保留在途回显——pendingEcho 存活、`ensurePendingEcho(turnsFromMessages(...))` 丢弃重置前内容且幂等重推、done 之后不再复活旧 prompt；(c) steer 消费回显后新梯落在其下（user 边界封顶回溯），round-1 梯 deep-equal 冻结快照。
- **`features/index.md`**：Turn 阶梯条目补挂 [N Steps 与 Say 成对] 链接（原 grep 无命中），并把「各自拥有自己的 `N Steps + Say`」修正为多对语义（一次提交可含多对，非空 Say 收合当前子轮、其后 reasoning 在 Say 之下开新梯，呈现 `[N Steps + Say]×n` 交替）。

## 瞬态失败溯源（TODO-2 结论）

23 次全量 vitest（20 循环 + 3 确认）仅 2 次失败（run #12/#13），均归因**并发写入**：本方子代理恰在 run #11-13 之间重写 `saypairs.e2e.test.js`（mtime 03:11:31 证据；路径哈希不可见、仅内容变化），此后连 12 次全绿。测试计数漂移 179→186 全部由两个新测试文件解释。原评审怀疑的 chat.dom.test.jsx「router `sessionSnapshots` hook 污染」系误记——实为 fetch-mock fixture map，无 `window.history` 跨测试共享。静态审计另发现两处**真实但当前良性**的隐患（fixture map 与 `consoleLog` 告警桶未在 `beforeEach` 重置，当前仅靠声明顺序幸免）；精确修法（beforeEach 补 `sessionSnapshots = {}` 与桶清零）已记录于 `/tmp/vitest-loop-report.md`，该文件并发会话所有，未代改。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| reduce 层 settle-before-append 顺序不变量 | `spa/src/reduce.order.test.js` (order-a/b/c)（新增） |
| 帧级 e2e：两回合 `[N Steps + Say]×2` 交替、各自冻结、梯 2 在 Say 1 之下 | `spa/src/saypairs.e2e.test.js` (a)（新增） |
| 帧级 e2e：transcript_reset 保留在途回显、done 退役不复活 | 同文件 (b)（新增） |
| 帧级 e2e：steer 消费开新梯、旧梯冻结不动 | 同文件 (c)（新增） |
| `turnHasSay` 注释收敛行为中性 | `spa/src/steps/reducer.test.js` 25 例回归通过 |

## 回归

vitest 186/186（17 文件，含并发会话在逧行）；`cargo fmt --check` 干净；`cargo clippy --workspace --all-targets -- -D warnings` 0 警告；`cargo test --workspace` 全绿（3997 passed / 0 failed，EXIT=0）；`scripts/check-spa-drift.sh` no drift（并发会话 dist 与其 src 同步，注释经 minify 中性）。

真机视觉验证（多回合交替 + 12px 间距、`/act_clear_context` 在途回显、steer/queue 拆梯）保持人工 TODO：`scripts/browser-acceptance.js` 需 playwright-core + 真 LLM 长流，本环境缺前者。
