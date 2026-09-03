# SPA step 阶梯三级下钻重构 + thinking 段内吸收

Commit: 9b999cf

## 背景

- 2539b78 的恒展开模型（静态 `≡ N steps` marker + 步行恒可见）在步数多时占屏过高；且 thinking 吸收只吃「严格尾随」think run——think → Say → tool 序列会在顶层残留游离 think turn。
- 目标模型（与 TUI 并行对齐，TUI 由另任务处理）：一个工具 turn 的 steps 气泡顶层只有一行可点击组行，展开后才是步骤；步内 thinking 直接可见；calls 收进聚合行；单个 call 再展开 input/output。有工具调用的 turn，thinking 只出现在 step 内部。

## 变更（SPA，`crates/web/spa/src/`）

- `stepsBlock.jsx`：`StepsContent` 根改为**一个** antd Collapse（size small、ghost、默认收起）作 L0 组行——label mono `❯ {n} step{s}`（n===1 单数）+ running Tag（processing，任一 call `output===null`）/ error Tag（red，非 running 且有 isError），收起时整个步梯不渲染；L1 每步一个 `❯ Step(k)` Collapse（步内含失败 call 挂红 error Tag，去掉 `· n calls` 后缀）；L2 步内 `step.thinking` 直接渲染（`💭 Thinking` 小标签行 + mono fontSize 12 pre-wrap `#8c8c8c` 段落，不再套 ThinkContent 的 ghost Collapse）+ calls 聚合 Collapse（ghost）label `❯ {m} function call{s}`；L3 每 call 一个原样保留的 `ToolContent`（input/output 展开语义不动）。`ThinkContent` 仍导出——transcript.jsx 的 `think` 角色（纯文本轮 thinking）在用；`subagentBlock.jsx` childLines 消费同一 turns 形状，零改动。
- `reduce.js` live 路径：`popTrailingThinking` 改名 `absorbSegmentThinking`——从尾往前吸收**本 user 段内全部** assistant think turn（`splice` 摘除、文本 earliest-first 拼接），跨过 assistant text turn（Say 原地保留为顶层气泡），遇到 user/sys/task/subagent/steps 等边界停；`mergeOrNewStep`/`stepBoundaryNeeded` 逻辑不变（尾步含 finished call 才开新步），Say 之后开新 steps turn 的语义保留。
- `reduce.js` 快照路径 `turnsFromMessages`：改为索引化消息/块循环 + lookahead 预扫（`stepToolAt` 每消息最后一个非 task tool_use 块下标、`toolAhead` 后向递推「下一 user 边界前是否还有非 task tool_use」）；assistant Say 只有在段内无后续工具轮时才独立 flush pending think（纯文本轮保持顶层 think turn），否则保持 pending 由后续轮首 tool_use 吸收——与 live 折叠对齐，reasoning-only message + 后续 tool message 的既有合并语义锁定。`task`（subagent）行为两侧均不动（仍扁平行）。
- `transcript.jsx`：仅 `BUBBLE_ROLES.steps → StepsContent` 映射与头注释更新；epoch 收起（Ctrl/Cmd+L、`⤒ 收起`）靠 remount 复位所有非受控 Collapse，新层级全部非受控，机制不变。

## 测试清单（规则 01/02：每行为有测试）

- `stepsBlock.dom.test.jsx` 重写为 11 用例：零点击只见组行（步行/thinking/聚合行/call 名/input/output 全不在文档）；点组行→步行出现；点步行→thinking 直接可见 + `❯ 1 function call` 聚合行；点聚合行→call 名可见；点 call→input/output 精确文本；Ctrl+L 与 `⤒ 收起` 全复位；running/error Tag 在组行（展开后步行另见 error Tag）；单复数（`❯ 1 step` / `❯ 2 function calls`）；Say 为 steps 之后独立气泡。
- `reduce.test.js` 41→48 用例：live (c) 重写（跨 Say 吸收）+ 新增 (c2) Say 两侧 think 拼接、(c3) user/sys 边界停吸收、(c4) 纯文本轮 think 保持顶层；快照新增 (j) reasoning-only+tool message 合并入步、(k) Say 前 reasoning 折入后续轮（live 对齐）、(l) reasoning+text 无工具保持现状、(m) user 边界停吸收；(a)(b)(d)–(i) 及既有快照/echo 契约用例不回归。
- `subagentBlock.dom.test.jsx`（childLines 对 steps 形状兼容）与其余 12 个测试文件全量回归通过。

## 回归

- `cd crates/web/spa && npm test`：**141 passed / 0 failed**（14 文件）。
- `scripts/build-spa.sh` 已重建并提交 `crates/web/spa/dist`（dist 为内嵌产物，随 UI 改动必须重建）。
