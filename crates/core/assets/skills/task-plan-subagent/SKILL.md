---
name: task-plan-subagent
description: 在 task-plan 的上线闭环规划基础上，把复杂任务拆分成 subagent 可以稳定交付的粒度，并将执行、集成、验收与验证全部委托给 subagent；父 agent 只负责任务拆解、调度和全局需求进展把控。Use when the user asks for subagent-based task planning, parallel agent execution, delegated implementation, or splitting every subtask into subagents after a launch-ready plan.
---

# Task Plan Subagent

## Overview

在 `task-plan` 的上线闭环规划之上，增加一个强约束：任务必须先拆到 subagent 可以稳定交付的粒度，然后每个可执行子任务都交给一个 subagent 执行。

父 agent 只作为 subagent 调度器，本身只做任务拆解、派发编排、结果回收和全局需求进展把控。执行、集成、验收、验证、发布准备检查等动作全部交给 subagent；父 agent 不得亲自执行这些动作，也不得把自己完成的检查当作验收结论。若当前环境没有可用 subagent 工具，必须明确说明执行受阻，并输出可直接派发的子任务清单。

## When To Use

- 用户明确要求使用 subagent、并行 agent、委派执行或“每个子任务交给 subagent”
- 复杂需求已经需要 `task-plan` 级别的上线闭环规划，同时执行面适合拆成多个独立工作单元
- 当前任务存在多个模块、多个验证面、多个文档或多个发布准备项，可以按 disjoint ownership 并行推进
- 用户要求先做任务拆分，再由多个 agent 分工交付

如果用户只是要普通上线闭环规划，优先用 `task-plan`。如果用户明确要求直接修复但没有授权使用 subagent，不要主动触发本 Skill。

## Relationship To task-plan

- 先继承 `task-plan` 的范围重建、现状审查、根因识别、闭环计划、线上或生产等价验证方案和遗漏复查要求。
- 可读取 `../task-plan/references/launch-closure-plan-checklist.md` 作为上线闭环检查基线。
- 再读取 [references/subagent-delegation-checklist.md](references/subagent-delegation-checklist.md)，把闭环计划拆成可派发任务。
- `task-plan-subagent` 的新增价值不是“计划更长”，而是让每个子任务具备明确 owner、写入范围、验收方式和回收协议。

## 拆分前对齐（question 工具）

会改变拆分与派发走向的未定事项必须先与用户对齐，不得自行假设后直接派发 subagent；无疑问则不强制提问。仓库、`rules/`、既有测试和 `AGENTS.md` 能回答的事实一律先查再定，不把提问当侦察手段。

触发条件（任一命中即提问，不得靠推断绕过）：

- 子任务边界二选一，或两个 subagent 的写入范围重叠、owner 归属有争议
- 并发波次与串行顺序牵涉不可逆动作：数据迁移、发布、外部依赖开通、凭据轮换
- 验收口径缺失：由哪个 verifier subagent、以什么证据判定该子任务完成
- 需要动用权限、环境、发布窗口或对外接口，而授权范围未明确

调用口径与 `task-plan` 一致：

- `question`（string，必填）：只放一个问题，一句话说清；不把多个问题合并进一次调用。
- `options`（string[]，可选）：不超过 4 个短候选答案，互斥且覆盖主流取舍；开放性问题省略该参数。
- 调用示例：`{"question": "支付回调改造与对账脚本改造能否并行？", "options": ["可并行，写集不相交", "必须串行，先回调后对账"]}`
- 工具结果即用户所选答案或原文答复，拿到答复再固化拆分契约；同一轮可对多个独立决策点分别调用（一次一个最关键问题），全部拿到答复后再派发第一波 subagent。
- 无交互能力、或答复被用户跳过（工具返回用户已跳过、自行判断）时：显式假设继续拆分——把推断逐条列入 `assumptions:` 清单，标注为对应 subagent 的验收前提，选最小意外解释；绝不静默编造验收标准或替用户拍板不可逆取舍。

提问只能由父 agent 自己完成：不得派一个 subagent 去替用户拍板，也不得把「等待用户答复」写成某个 subagent 的任务。

## Parent Agent Contract

父 agent 只保留这些调度职责：

- 读取仓库约束、当前 diff、关键上下文和发布要求
- 定义目标、非目标、成功标准、失败标准和 blocker
- 拆分 dependency graph、并发波次和每个 subagent 的 ownership
- 派发 explorer、worker、integration worker、verifier 或 release-check subagent
- 回收每个 subagent 的状态、证据、blocker 和进度，维护全局需求进展
- 发现跨 subagent 冲突、证据缺口或遗漏后继续拆分并重新派发
- 汇总 subagent 的验收结论、上线判断和最终回复

强约束：父 agent 不得直接执行或验收这些任务：

- 模块实现、测试补齐、脚本修复、UI 调整、文档维护、迁移演练、回归验证、发布准备检查
- 独立探索问题、候选方案比较、影响面审计
- 集成修复、冲突处理、端到端串联、最终验证、上线前验收和发布检查
- 将缺少 verifier 或 acceptance subagent 证据的子任务标为完成

允许的父 agent 动作只限于：读取必要上下文、就不可逆拆分决策用 `question` 与用户对齐、生成派发 prompt、检查回传格式和证据是否缺失、决定是否继续派发。父 agent 的检查不等同于验收通过；任何实现、验证、集成、发布相关命令、文件修改、数据操作、发布动作或验收判断都必须由对应 subagent 完成。

## Stable Subtask Granularity

每个 subagent 子任务必须满足：

- `目标单一`：能用一句话说明交付物，避免“顺便把相关问题都看看”
- `边界清晰`：有明确读写文件、模块、接口或验证面
- `写集隔离`：并行 worker 不应修改同一文件；无法隔离时拆成顺序波次
- `上下文充分`：提供必要背景、约束、相关路径、验证命令和禁止事项
- `可独立验收`：有完成定义、证据要求、失败信号和回传格式
- `规模可控`：单个任务应能在一轮 subagent 工作中稳定完成；过大就继续拆
- `风险显式`：涉及数据库、环境变量、敏感信息、发布、删除数据或生产操作时明确护栏

不合格的子任务要继续拆分，直到每个子任务可以被一个 subagent 独立交付。

## Workflow

### 1. 建立上线规划基线

- 读取仓库指令、memory 文档、当前 diff、相关模块和验证入口。
- 按 `task-plan` 的方式明确问题范围、上线目标、非目标、已确认问题、风险假设和外部 blocker。
- 如果问题范围还无法判断，先派发 explorer subagent 做独立影响面审计，再回收证据后继续拆分。

### 2. 拆分为 subagent dependency graph

- 把总目标拆成 deliverables，再拆成 subagent 子任务。
- 标注每个子任务的依赖、写入范围、验证方式和交付证据。
- 将可并行任务放入同一波次；存在文件冲突、数据依赖或设计依赖的任务拆成后续波次。
- 每个执行项必须有一个 subagent owner；不能出现无人负责的 TODO。

### 3. 派发子任务

- 对探索类任务使用 explorer subagent；对实现、修复和文档类任务使用 worker subagent；对验证和验收类任务使用 verifier 或 acceptance subagent。
- 每个 prompt 必须包含：
  - 背景与目标
  - 明确 ownership 和禁止修改范围
  - 必须遵守的仓库约束
  - 具体交付物和回传格式
  - 验证命令、证据要求和失败时要报告的内容
- 告诉 worker：当前代码库可能有其他人并行修改，不要 revert 他人改动，遇到冲突要适配并报告。
- 不要把同一 unresolved 子任务重复派发给多个 subagent。

### 4. 回收、再派发和进度把控

- 回收每个 subagent 的最终回复、改动路径、验证证据和遗留问题。
- 检查回传是否包含状态、证据、blocker、越界修改和风险说明；缺失时继续派发补充执行或验收任务。
- 对每个可交付子任务派发 verifier 或 acceptance subagent 验收；父 agent 不直接给出验收通过结论。
- 如需跨任务集成，必须作为新的 integration subagent 任务派发；集成后的复验必须再派发 verifier subagent。
- 对失败子任务先分析根因，重新拆分或修正 prompt 后再派发，不用兜底隐藏问题。

### 5. 派发最终验证和汇总上线判断

- 派发 verifier subagent 运行与影响面匹配的测试、构建、lint、脚本验证、UI 验收或线上/生产等价验证。
- 派发 verifier 或 release-check subagent 按 `task-plan` 的遗漏复查面检查边界、异常流、兼容性、数据、配置、安全、发布和观测。
- 父 agent 只汇总 subagent 的验证证据、未完成项和上线判断；证据不足时必须继续派发或明确标记 blocker。

## Output Schema

最终回复至少覆盖：

- `问题与范围`：目标、非目标、上线标准和当前 blocker
- `现状审查`：已确认问题、风险假设、外部依赖
- `Subagent 拆分矩阵`：子任务、owner 类型、写入范围、依赖、验收方式、状态
- `派发波次`：哪些任务并行，哪些任务顺序执行，为什么
- `Subagent Prompt 摘要`：每个子任务的目标、约束、交付物和验证要求
- `回收与集成`：每个 subagent 的结果、改动路径、证据、冲突、后续派发和处理结论
- `验证方案`：由 verifier subagent 执行的本地、生产等价或线上实质校验步骤和判定标准
- `遗漏复查`：未覆盖项的纳入、blocker 或排除原因
- `最终判断`：基于 subagent 验收证据判断是否达到可上线或可交付状态，仍缺什么

## Notes

- repo 指令始终优先于本 Skill。
- 父 agent 只能做任务拆解、调度和全局需求进展把控；执行与验收动作必须全部交给 subagent。
- Subagent 并行不是目标，稳定交付才是目标；无法保证写集隔离时使用顺序波次。
- 不得把缺少证据的子任务标成完成。
- 不得将敏感信息写入 git 跟踪文件；需要凭据时使用环境变量或仓库指定的安全来源。
- 不得执行删除生产数据等危险动作，除非用户复核后明确授权。
