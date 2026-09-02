---
name: summary
description: Task retrospective at any checkpoint (done/paused/handoff). Produces a structured recap — original requirements, what was done, how completion was verified (with evidence), and remaining optimization space. Read-only, no commits or code edits.
---

# summary —— 任务回顾契约

## 角色
任务回顾契约。在任意节点（完成 / 暂停 / 交接 / 被问「这活干得怎样」）介入，对本次任务做一次结构化回顾：需求是什么、做了哪些事、怎么验证做完的、还有哪些优化空间。

> **只读**：本 skill 不修改代码、不提交、不推送。需要修改时回 `do-and-done` / 实现循环；需要提交时用 `submit`；需要上线评审用 `review`。

## 何时使用
- 自评「做完了」或达到一个里程碑节点，需要产出一份回顾给人看。
- 任务暂停 / 中断后恢复，快速重建「之前到哪了」的上下文。
- 交接给他人 / subagent 时，作为 handoff 的结构化说明。
- 被问「这次任务需求是什么 / 做了啥 / 怎么验证的 / 还有啥能改进」。

## 输入
- 本次任务的初始 prompt / task-plan 的问题范围、闭环计划与证据——需求来源。
- 工作区实际状态：`git status`、`git diff`、`git log --oneline <base>..HEAD`。
- 会话历史：做了哪些工具调用、改了哪些文件、跑了哪些验证。
- 已有的验证证据（测试输出、构建结果）—— **优先引用真实证据，不臆造**。

## 输出（固定四段结构）
按以下结构产出回顾，每段都基于事实（git diff / 命令输出 / 文件改动），不得臆测：

### 1. 任务需求（本次要解决什么）
- 一两句话概括原始目标 / 验收标准。
- 如需求中途变化，简述变化点。
- 标注需求中隐含的约束（性能 / 兼容 / 测试要求等）。

### 2. 做了哪些事情（实际变更）
- 列关键改动（文件 / 模块 / 行为），按逻辑分组，不逐行堆砌。
- 引用 `git diff --stat` 或文件路径作为依据。
- 区分「核心交付」与「附带 / 范围外」的改动。

### 3. 怎么验证做完的（验证方式 + 证据）
- 列实际执行的验证命令及其结果（测试数、构建 / 类型检查状态）。
- 证据必须可复现：给出命令，而非「感觉没问题」。
- 标注尚未覆盖或无法验证的部分（诚实，不伪绿）。

### 4. 存在的优化空间（还能怎么更好）
- 代码质量：可读性 / 重复 / 抽象边界 / 文件是否触限（新增 ≤400 行，迭代中 ≤800 行）。
- 健壮性：边界条件 / 错误处理 / 并发 / 性能隐患。
- 测试：覆盖缺口 / 分层是否合理（unit / integration / e2e）/ 是否需要补充。
- 文档与记忆：`agents/*` / `features/*` 是否需要同步。
- 按优先级排序（高 / 中 / 低），给出可执行的下一步。

## 原则
- **事实优先**：需求、改动、验证都从 git / 命令输出取证，不靠记忆臆断。
- **诚实**：没做的、没验证的、有风险的，如实标出，绝不粉饰。
- **不重复造清单**：复用 task-plan 的闭环计划与 review 的上线结论，summary 只做回顾与梳理。
- **精炼**：回顾是给人快速建立全貌的，避免冗长；关键证据给命令 + 结果摘要即可。

## 与其它 skill 的衔接
- 完成实现（`do-and-done`）后，summary 产出回顾；review 做 go-live 评审；submit 做提交。
- summary 是只读回顾，不替代 review 的逻辑评审与 go-live 裁决，也不替代 submit 的提交动作。
- 触及区 memory（`agents/*`、`features/*`）若需更新，按 `repo-local-memory` 处理，summary 本身不改文件。
