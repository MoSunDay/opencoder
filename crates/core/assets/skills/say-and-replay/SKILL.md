---
name: say-and-replay
description: Read-only alignment snapshot at any checkpoint. Restates the original goal/acceptance criteria, replays completed TODOs with evidence, exposes current blockers (and what/who is needed to clear them), and lists the remaining TODOs to fully close the loop. Orthogonal to the main chain — use it to align progress, surface blockers, and support handoff/resume. Never edits code or commits.
---

# say-and-replay —— 复述与回放（对齐快照）

## 角色
对齐快照契约。在任意检查点介入，把当前任务「**说清楚原始需求目标本身 → 回放已做的 TODO 及验收证据 → 暴露当前卡点 → 列出后续完全闭环所需 TODO**」一气呈现，用于对齐进度、暴露阻塞、支撑交接 / 恢复。

> **只读**：本 skill 不修改代码、不提交、不推送、不重排 STATUS 块。它只做一次结构化复述 + 回放。需要实现时回 `do-and-done`；需要规划 / 重排时回 `task-plan`；需要提交时用 `submit`；需要回顾优化空间用 `summary`。

## 何时使用
- 需要对齐「这活到底要什么 / 做到哪了 / 卡在哪 / 还差什么」时（无论中途还是节点）。
- 任务中断 / 暂停后恢复，需要快速重建上下文。
- 交接给他人 / subagent 前，作为结构化 handoff。
- 被问「当前进度如何」「有没有卡住」时。
- 多轮长任务中，每隔一段时间主动对齐一次，防止偏航。

## 输入
- **原始需求**：用户最初的 prompt / issue / 任务描述 —— `goal` 取自**这里**（原始诉求本身），不是派生的 STATUS goal。
- **STATUS 块**（若会话中存在 task-plan 产出的 STATUS 块）：作为 `done` / `doing` / `remaining` 的数据源——**有则复用，不另起清单**。
- 工作区实际状态：`git status`、`git diff`、`git log --oneline <base>..HEAD`。
- 会话历史：已做的工具调用、改动文件、验证结果——**取证而非臆断**。

## REPLAY 块（固定输出格式，每次必须产出）
```
## REPLAY
goal: <复述原始需求目标本身 + 验收标准 —— 来自用户原始 prompt，而非派生的 STATUS goal>
progress: <completed 数>/<总数>   ← 仅计入有验收证据的 completed
done:                          ← 已完成且有证据的 TODO
  - <已完成 TODO> | accept: <验收标准> | evidence: <命令+结果 / file:line / 日志摘要>
doing:                         ← 进行中或证据尚不充分的项
  - <进行中 TODO> | reason: <为何未完成 / 证据缺什么>
blocked:                       ← 当前卡点（无则写 none）
  - <卡点描述> | blocks: <被它阻塞的 TODO 清单> | need: <解除卡点需要什么 / 等谁>
remaining:                     ← 后续完全闭环所需的 pending TODO
  - <待办 TODO> | accept: <验收标准> | impact: <受影响模块 / none>
verdict: <on-track | at-risk | blocked | done>   ← 一句话对齐判定 + 理由
```

### 字段语义（精确映射用户诉求）
- **goal（复述原始需求目标本身）**：逐字 / 贴近地复述用户**最初提出**的需求目标 + 验收标准。不是 STATUS 块里派生改写后的 `goal`，而是回溯到「用户到底要什么」。需求中途变化则一并标出。
- **progress（进度）**：`completed/total`。只计入**有验收证据**的 completed；无证据项归入 `doing`，不得虚高进度。
- **done（已做的回放）**：逐条列已完成 TODO + 验收标准 + **可追溯证据**（命令 + 结果摘要、`file:line`、日志）。证据不可复现的不得计入。
- **doing（进行中）**：正在做但未完成、或证据不足的项，附 `reason:` 说明为何卡在这一步。
- **blocked（暴露当前卡点）**：明确列出当前阻塞，附 `blocks:`（它阻塞了哪些 TODO）和 `need:`（解除需要什么 / 等谁 / 等什么）。**无卡点写 `none`，诚实暴露，不粉饰**。
- **remaining（后续闭环 TODO）**：列清要完全闭环还差哪些 pending TODO，每条附 `accept:` + `impact:`（与 task-plan 的 accept/impact 同义）。
- **verdict（对齐判定）**：`on-track`（按计划推进）/ `at-risk`（有风险但未死锁）/ `blocked`（被卡点死锁，需外部介入）/ `done`（已完全闭环）。一句话给出判定 + 理由。

## 原则
- **复述而非改写目标**：`goal` 回溯用户**原始**需求，不擅自引申、缩减或重定义。需求变化时如实标注变化点。
- **证据驱动**：`done` 的每条都有可追溯证据（命令 + 结果 / `file:line`）；没有证据不算做完，归入 `doing` 并写明 `reason`。
- **诚实暴露卡点**：`blocked` 不回避、不弱化——把「卡了什么、阻塞了谁、需要什么」说清楚，是本 skill 的核心价值。
- **复用 STATUS 块、不另起清单**：会话中存在 task-plan 的 STATUS 块时，`done`/`doing`/`remaining` 以其为数据源直接回放，不重复造清单；无 STATUS 块时从 transcript + git 现场取证。
- **只读对齐，不动手**：只产出 REPLAY 快照；发现问题要修复→`do-and-done`，要重排→`task-plan`，要回顾优化→`summary`。

## 与其它 skill 的衔接
- **与 task-plan**：消费其 STATUS 块作 `done`/`doing`/`remaining` 数据源；但 `goal` 始终回溯用户原始需求，两者对照可发现「派生目标是否偏离原始诉求」。需要重排 TODO 时回 `task-plan`。
- **与 do-and-done**：`doing`/`blocked` 暴露的未决项，交回 `do-and-done` 推进；卡点解除后重新回放更新 REPLAY。
- **与 summary**：本 skill 是「当前对齐快照」（聚焦卡点 + 后续闭环），summary 是「完整回顾」（聚焦做了啥 + 优化空间）。两者正交、平级，各司其职。
- **与 review**：review 评审是否达 go-live 标准（带 gate 强制），本 skill 只做进度对齐快照、不做 gate 判定。
- **与 submit**：本 skill 不提交；`verdict: done` 仅表示已闭环，提交动作仍由 `submit` 按其暂停协议执行。
