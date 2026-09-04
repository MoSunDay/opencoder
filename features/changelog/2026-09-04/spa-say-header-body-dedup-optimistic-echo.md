Commit: af71944

# SPA Say 头部/正文渲染修复（间距 + 首行去重）与直播回显去重

## Context

`Say(n steps)` 合并头部行（`features/changelog/2026-09-04/spa-say-row-running-label-merge.md`）合入 WIP 后用户报告三处缺陷：② 头部行与正文挤压（say wrapper 仅 marginTop 8px，`❯ Say(1 step): xxx` 行与正文首行视觉粘连，TUI 是头部后插一空行）；③ 单行 Say 的正文与头部 preview 一字不差重复渲染；直播期间乐观回显 + 服务端 `steer_consumed` 帧推同文本 user 回显导致用户输入显示两条、done 重建后才收敛为一条。

## Change Summary

- **`spa/src/sayText.js`（新增，纯 JS 无 DOM）**：`sayPreview` 自 stepsBlock.jsx 迁出并导出（拼接口径不变：非 image 文本 parts 拼接后首个非空行 trim）；新增 `sayBodyParts(say, preview)` —— 按同一拼接口径定位正文首个非空行，trim 与 preview 相等则跳过该行（preview 无截断，按完整首行比较；不等则正文全量保留），首行跨 parts（夹 image 标记行）由 carry 承接，去完后纯空白的文本 part 整个丢弃（单行 Say → 空数组），think/sys/image 部分原样保留，输入不修改。
- **`spa/src/stepsBlock.jsx`**：`sayPreview` 改为从 sayText.js 导入（头部与正文去重共用单一口径，永不漂移）；L0 标签逻辑不变。
- **`spa/src/transcript.jsx` `AssistantTurnContent`**：Say 正文块整体包一层 `marginTop: 16`（TUI「头部后插一空行」的对齐）；正文先过 `sayBodyParts` 去重，为空则整个正文块不渲染——不残留间距或空节点；正文内多个部分间保留 8px（首个不重复加）。
- **`spa/src/reduce.js`**：`withUserTurn(state, text, optimistic?)` 第三参打 `optimistic:true` 标记（sendLocal initialTurns 与 sendRemote 的 `withUserTurn` 调用均传 true）；`queue_consumed`/`steer_consumed` 折叠时若末位 turn 是同文本带标记的乐观回显 → 不 push 第二条，改为返回去掉标记的新数组（成为权威回显）；否则维持原 push。`pendingEcho` 记账语义不变，applySeq 水位/重放语义未触碰。
- **`spa/src/chat.jsx`**：移除临时调试补丁（`window.__ocFold`/`__ocFoldLog`），`onFrame` 恢复为 `setStream((s) => reduceFrame(s, f, Date.now()))`，queue/steer 的 setQueueVersion 与 transcript_reset 的 reloadAfterDone 原样保留。
- 服务端帧序佐证（非改动）：`runner/steer.rs` 中 `SteerConsumed` 先于控制命令应用（ClearContext→`TranscriptReset`）发出，故 `/act_clear_context <tail>` 路径的乐观标记总在 reset 重建之前被消费，无重建后重复风险。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| preview 拼接口径（首非空行 trim、image 不算、空 Say 空串） | `sayText.test.js`（新增） |
| 正文首行去重：多行跳首行 / 单行空数组（含尾换行、前后空白）/ 首行 != preview 全量保留 / think/sys/image 原样 / 跨 parts 拼接 / 不改输入 | `sayText.test.js`（新增） |
| DOM：多行 Say 正文跳过 preview 首行且与头部 16px 间距 | `stepsBlock.dom.test.jsx`（新增） |
| DOM：单行 Say 无正文块、无残留间距/空 Typography 节点 | `stepsBlock.dom.test.jsx`（新增） |
| DOM：`N Steps + Say` 单气泡 + 文档顺序（Say 改多行，避开与去重冲突） | `stepsBlock.dom.test.jsx`（更新） |
| reduce：同文本 steer/queue_consumed 折叠乐观回显且标记被清、pendingEcho 不变 | `reduce.parity.test.js`（新增） |
| reduce：不同文本 steer_consumed 正常 push（旧回显标记保留） | `reduce.parity.test.js`（新增） |
| reduce：无标记同文本回显维持原 push（权威回显不去重） | `reduce.parity.test.js`（新增） |
| live/snapshot 一致性：spec 帧序逐帧折叠 vs 对应快照消息走 turnsFromMessages，Say 头部标签序列 `Say(1 step)/Say(2 steps)/Say(2 steps)` 与 steps 计数 [1,2,2] 完全一致（每子轮计数不累加） | `reduce.parity.test.js`（新增） |
| 端到端：提交→同文本 steer_consumed 仍只有一条 user 泡，不同文本追加第二条 | `chat.dom.test.jsx`（新增） |

## 回归

- `cd crates/web/spa && npm test`：19 文件 **212 用例全绿**（基线 196 + 净增 16，1 个既有用例按新契约更新）。
- `cd crates/web/spa && npm run build`：重建 `spa/dist`（index.html + static/app.js + static/app.css，固定文件名白名单机制不变）。
- 未提交任何 git commit；未触碰 Rust 侧与 reduce.js 的 applySeq/重放语义、bubbleItems.js 分组规则。

## 偏离说明

- 规格给出的快照消息形状末轮为 "assistant [text]"，但帧序里第三轮含 `reasoning_delta`（会落库为该 assistant 消息的 reasoning 块）；快照不带它时 turnsFromMessages 产出 `Say(1 step)` 而直播路径为 `Say(2 steps)`，两条路径必然对不齐。一致性用例的快照末轮因此带 reasoning 块（与帧序真正对应），契约断言不受影响。
- `chat.jsx` 751 行（改动前 746，本次 +5 且为注释/换行），超出 400 行新文件上限但未超 800；按「仅当本次改动导致超限才拆分」保持现状。`reduce.js` 718 行同理（WIP 基线 698）。
