# Local Memory Minimal Templates

This file is an optional appendix, not the main rule source.
Follow `SKILL.md` and repository-specific memory rules first.
It covers local-memory docs in this skill's output scope and intentionally does not define a default template for `AGENTS.md`.

Use these templates when:
- creating a new local-memory doc from scratch
- repairing a doc whose structure is weak or inconsistent
- you need a minimal scaffold and do not want to invent section layout

These are starter skeletons, not mandatory exact headings.
Default to Simplified Chinese headings unless repository rules require otherwise.
Always keep the first line as `Commit: <documentation baseline git commit sha>`.

---

## Template: `agents/{module}/index.md`

```md
Commit: <documentation baseline git commit sha>
# <模块名>

## 职责
- <模块负责什么>

## 边界
- 负责：<拥有的职责 / 数据 / 流程>
- 不负责：<明确不归属的内容>

## 关键设计
- <核心抽象 / 关键约束 / 设计选择>

## 核心链路
1. <主链路步骤 1>
2. <主链路步骤 2>
3. <主链路步骤 3>

## 依赖与接口
- <外部依赖 / 对外接口 / 输入输出>

## 关联模块
- [<相关模块>](../<related-module>/index.md)
```

`agents/{module}/{submodule}/index.md` can usually reuse the same skeleton with a narrower submodule title, boundary, and links.

---

## Template: `agents.md`

```md
Commit: <documentation baseline git commit sha>
# <仓库名> Overview

## Overview
- <仓库当前逻辑全貌>

## Agent 模块索引
- [<模块 A>](./agents/<module-a>/index.md)
- [<模块 B>](./agents/<module-b>/index.md)

## Features 索引
- [features/index.md](./features/index.md)
```

Keep this file as a repository-level logic map only.
Do not expand it into implementation details, changelog history, or repeated child-doc content.

---

## Template: `features/{feature}/index.md`

```md
Commit: <documentation baseline git commit sha>
# <功能名>

## 能力概述
- <用户 / 调用方可感知的能力>

## 触发方式
- <页面 / API / 命令 / 事件 / 工作流>

## 行为与规则
- <核心业务规则>

## 关键状态与异常
- 状态：<关键状态>
- 异常：<主要失败面 / 限制>

## 关联逻辑模块
- [<相关模块>](../../agents/<module>/index.md)
```

---

## Template: `features/index.md`

```md
Commit: <documentation baseline git commit sha>
# Features Index

## 能力分组
- [<功能 A>](./<feature-a>/index.md)
- [<功能 B>](./<feature-b>/index.md)

## Changelog
- [changelog](./changelog/)
```

Keep this file as a repository-level capability map only.
Do not enumerate every changelog entry here unless repository rules explicitly require it.

---

## Template: `features/changelog/YYYY-MM-DD/{topic}.md`

```md
Commit: <documentation baseline git commit sha>
# <变更主题>

## Context
- <为什么会有这次变更>

## Change Summary
- <改了什么>

## Impact Surface
- <影响到的能力 / 模块 / 接口 / 行为>

## Notes / Compatibility
- <兼容性 / 迁移 / 风险 / 注意事项>

## Related Docs
- [<相关 feature 文档>](../../<feature>/index.md)
- [<相关 agents 文档>](../../../agents/<module>/index.md)
```
