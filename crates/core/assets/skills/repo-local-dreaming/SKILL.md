---
name: repo-local-dreaming
description: Periodic memory-consolidation pass over repository local memory (agents.md, agents/*, features/* — changelog excluded). Review the current code baseline plus the change timeline, merge redundant claims, prune stale facts, and keep a current-state snapshot. Hard structural limits: max 400 lines per md file (split by semantic boundary when exceeded), max 10 files per directory (cluster into subdirectories), max 10 subdirectories per level (fan out another level recursively).
---

# repo-local-dreaming —— 记忆整理（做梦）契约

## 角色
周期性「做梦」整理仓库本地记忆：根据**现状**（代码基线）与**时间线**（changelog/git log）回顾全部记忆文档，整合冗余、修剪过期、保留当下状态快照。与迭代内记忆维护的分工：迭代内维护是**每次迭代的 repair-on-touch 最小更新**；本 skill 是**低频全量整理/固化**，二者不互替。**绝不改动 changelog**（`features/changelog/*` 是唯一时间线，永不合并、重写或删除既有条目；做梦产物是否记新 changelog 条目由用户决定，本 skill 不自动写）。

> **只读代码、只写记忆**：不修改业务代码、不跑构建（验证记忆主张时可只读检查代码）。

## 输入
- 当前代码基线：`git rev-parse HEAD`。
- 记忆树全量：`agents.md`、`agents/*`、`features/index.md`、`features/*`。
- 时间线参考：changelog 日期目录与 `git log`（只读参考、不修改）。

## 做梦四步

### 1. 盘点与漂移检测
- 枚举全部记忆文件 + 行数 + 每目录文件数。
- 对每份稳定文档抽样主张与代码对照（证据优先级同迭代内记忆维护：代码/接口 > 测试 > 既有文档）。

### 2. 整合
- 同一事实多处重复 → 保留最强锚点（最贴近对应代码的那份），其余改为相对链接。
- 跨文档矛盾 → 以代码为准裁决，败方改链接。
- 过期主张直接删除（不保留「曾经…」式历史叙述——历史归 changelog）。

### 3. 快照固化
- 语义真正收敛（有实质合并/修剪/拆分）的文档刷新 `Commit:` 基线行到当前 HEAD；不为刷新而刷新。
- 顶层索引（`agents.md` / `features/index.md`）仅在逻辑图/能力图实际变化时更新。

### 4. 结构守护
- 按下表逐项检查并执行超限动作。
- 拆分/聚簇后修复全部相对链接（含反向引用）。

## 结构硬约束

| 对象 | 上限 | 超限动作 |
|---|---|---|
| 单个 .md 文件 | 400 行 | 按语义边界拆子文档，父级留概览 + 索引，细节下沉 |
| 目录直接子 .md 数 | 10 | 按功能边界聚簇为子目录 |
| 任一层级子目录数 | 10 | 继续向上派生新层级，上层留索引（规则递归适用） |

注：`features/changelog/` 的日期目录天然按日分桶，不受「10 子文件」约束（时间线结构不动）；changelog 单条目仍受 400 行约束。

## DREAM 块（固定输出格式）
```
## DREAM 快照
- 基线: <HEAD sha>
- 盘点: <N> 文档 / <M> 行；冗余 <a> · 过期 <b> · 漂移 <c> · 待拆 <d>
- 整合: <合并/去重/裁决点列表>
- 固化: <刷新 Commit: 基线的文档列表>
- 结构: <拆分/聚簇/派生动作 + 行数/文件数前后对比>
- 未动: changelog（时间线永不改写）· <其它有意跳过的文档及原因>
```

## 原则
- **现状优先**：快照 ≠ 历史重建。
- **changelog 是唯一时间线**。
- **整合是去冗余不是压信息**：删重复不删事实。
- **结构上限是硬性的**：宁拆勿超。
- **语言策略沿用迭代内记忆维护**：简体中文正文，路径/符号/sha 原样。

## 与其它 skill 的衔接
迭代内最小更新 → 本 skill（周期全量整理）→ 任务级回顾；整理产物需要提交时走提交流程。
