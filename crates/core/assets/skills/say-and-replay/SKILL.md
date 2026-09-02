---
name: say-and-replay
description: Read-only alignment snapshot at any checkpoint with a mandatory five-question recap. Restates the original goal/acceptance criteria, replays completed TODOs with verify-method + evidence, quantifies progress (completed/total + percent), exposes all encountered blockers (resolved and open, plus what/who is needed to clear the open ones), and lists the remaining TODOs to fully close the loop. Orthogonal to the main chain — use it to align progress, surface blockers, and support handoff/resume. Never edits code or commits.
---

# say-and-replay —— 复述与回放（对齐快照）

## 角色
对齐快照契约。在任意检查点介入，**五问必答**——① 原始需求目标是什么（复述）② 做了哪些事情、做到了多少（回放 + 完成度 completed/total + 百分比）③ 过程中遇到了什么卡点（含已解除）④ 每个完成点怎么验证的、证据是什么 ⑤ 下一步 TODO——把当前任务「**说清楚原始需求目标本身 → 回放已做的 TODO 及验证方式与证据 → 暴露全程遇到的卡点与当前卡点 → 列出后续完全闭环所需 TODO**」一气呈现，用于对齐进度、暴露阻塞、支撑交接 / 恢复。

> **只读**：本 skill 不修改代码、不提交、不推送、不重排 STATUS 块。它只做一次结构化复述 + 回放。需要实现时回实现循环；需要规划 / 重排时回规划；需要提交时走提交流程；需要回顾优化空间做任务回顾。

## 何时使用
- 需要对齐「这活到底要什么 / 做到哪了 / 卡在哪 / 还差什么」时（无论中途还是节点）。
- 任务中断 / 暂停后恢复，需要快速重建上下文。
- 交接给他人 / subagent 前，作为结构化 handoff。
- 被问「当前进度如何」「有没有卡住」时。
- 多轮长任务中，每隔一段时间主动对齐一次，防止偏航。

## 输入
- **原始需求**：用户最初的 prompt / issue / 任务描述 —— `goal` 取自**这里**（原始诉求本身），不是派生的 STATUS goal。
- **STATUS 块**（若会话中存在规划阶段产出的 STATUS 块）：作为 `done` / `doing` / `remaining` 的数据源——**有则复用，不另起清单**。
- 工作区实际状态：`git status`、`git diff`、`git log --oneline <base>..HEAD`。
- 会话历史：已做的工具调用、改动文件、验证结果——**取证而非臆断**。

## REPLAY 块（固定输出格式，每次必须产出）
```
## REPLAY
goal: <复述原始需求目标本身 + 验收标准 —— 来自用户原始 prompt，而非派生的 STATUS goal>
progress: <completed 数>/<总数>（<0-100>%，向下取整）   ← 仅计入 verify+evidence 俱全的 completed
done:                          ← 已完成且有验证方式+证据的 TODO
  - <已完成 TODO> | accept: <验收标准> | verify: <怎么验证的：命令 / 方法> | evidence: <当次证据：命令+结果 / file:line / 日志摘要>
doing:                         ← 进行中或证据尚不充分的项
  - <进行中 TODO> | reason: <为何未完成 / 证据缺什么>
encountered:                   ← 全程遇到的卡点（含已解除；无则 none）
  - <卡点描述> | status: <resolved | open> | resolution: <解除方式；open 则写 pending>
blocked:                       ← 当前卡点（无则写 none）
  - <卡点描述> | blocks: <被它阻塞的 TODO 清单> | need: <解除卡点需要什么 / 等谁>
remaining:                     ← 后续完全闭环所需的 pending TODO
  - <待办 TODO> | accept: <验收标准> | impact: <受影响模块 / none>
verdict: <on-track | at-risk | blocked | done>   ← 一句话对齐判定 + 理由
```

### 字段语义（精确映射用户诉求）
- **goal（复述原始需求目标本身）**：逐字 / 贴近地复述用户**最初提出**的需求目标 + 验收标准。不是 STATUS 块里派生改写后的 `goal`，而是回溯到「用户到底要什么」。需求中途变化则一并标出。
- **progress（进度）**：`completed/total` + 百分比（`floor(completed 数/总数 × 100)`）。只计入**有验收证据**的 completed；无证据项归入 `doing`，不得虚高进度。
- **done（已做的回放）**：逐条列已完成 TODO + 验收标准 + **验证方式（`verify`）** + **可追溯证据（`evidence`）**。`verify` 回答「怎么验证的」（跑了什么命令 / 用了什么方法），`evidence` 回答「证据是什么」（命令 + 结果摘要、`file:line`、日志）。**两者缺一不计入 completed**，归入 `doing` 并写明 `reason`。
- **doing（进行中）**：正在做但未完成、或证据不足的项，附 `reason:` 说明为何卡在这一步。
- **encountered（全程遇到的卡点）**：过程中遇到的**所有**卡点，含已解除的——已解除标 `status: resolved` 并在 `resolution:` 写明解除方式（怎么解除的）；未解除标 `status: open`、`resolution: pending`（细节在 `blocked` 展开）。无则写 `none`。
- **blocked（暴露当前卡点）**：明确列出当前阻塞，附 `blocks:`（它阻塞了哪些 TODO）和 `need:`（解除需要什么 / 等谁 / 等什么）。**无卡点写 `none`，诚实暴露，不粉饰**。
- **remaining（后续闭环 TODO）**：列清要完全闭环还差哪些 pending TODO，每条附 `accept:` + `impact:`（与闭环计划的 accept/impact 同义）。
- **verdict（对齐判定）**：`on-track`（按计划推进）/ `at-risk`（有风险但未死锁）/ `blocked`（被卡点死锁，需外部介入）/ `done`（已完全闭环）。一句话给出判定 + 理由。

## 原则
- **五问必答**：目标复述 / 做了什么 / 遇到的卡点 / 怎么验证+证据 / 下一步 TODO，五者缺一不可——任一缺失或空泛（无证据）即 REPLAY 不完整，必须补齐后再产出。
- **复述而非改写目标**：`goal` 回溯用户**原始**需求，不擅自引申、缩减或重定义。需求变化时如实标注变化点。
- **证据驱动**：`done` 的每条都有验证方式 + 可追溯证据（命令 + 结果 / `file:line`）；没有证据不算做完，归入 `doing` 并写明 `reason`。
- **诚实暴露卡点**：`encountered` + `blocked` 不回避、不弱化——把「遇到过什么、卡了什么、阻塞了谁、需要什么」说清楚，是本 skill 的核心价值。
- **复用 STATUS 块、不另起清单**：会话中存在规划产出的 STATUS 块时，`done`/`doing`/`remaining` 以其为数据源直接回放，不重复造清单；无 STATUS 块时从 transcript + git 现场取证。
- **只读对齐，不动手**：只产出 REPLAY 快照；发现问题要修复→转实现循环，要重排→转规划，要回顾优化→转任务回顾。

