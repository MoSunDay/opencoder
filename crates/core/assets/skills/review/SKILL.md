---
name: review
description: 围绕五个必答问题评审需求交付：原始需求目标、做了哪些事情及完成度、卡点、逐项验证+证据、下一步 TODO，并以当次实跑证据裁决 go-live ready 或 not ready。Use when the user asks for a review of completed or in-progress work, an assessment of completion percentage, verification evidence, blockers and remaining gaps, or whether a requirement is ready to ship.
---

# Review

## Overview

评审 = 回答五个问题 + 裁决上线结论。答好五问本身就是产出，没有固定输出模板。

## 问一：原始需求目标

回溯用户的原始需求，逐字复述目标与验收标准，不引申、不缩减、不重定义；需求有变化时如实标注变化点。

## 问二：做了哪些事情及完成度

盘点实际改动，给出完成度 = `completed/total` + 百分比（向下取整）。完成的每一项都必须同时有验证方式与当次证据，缺一不计入完成。

## 问三：卡点

诚实暴露卡点：遇到过什么、卡在哪里、阻塞了谁、需要什么（授权、审批、权限、凭证、外部依赖）。无卡点也要显式说明「无」。

## 问四：逐项验证+证据

对每一项完成点逐项回答：怎么验证、证据是什么。证据必须是当次实跑（当场重跑命令、复测断言，不引用历史输出）；**没有证据 = 没有通过**。

## 问五：下一步 TODO

列出距离完全闭环还缺的事项，按优先级排序，写明每项的落地路径与所需条件。

## 上线结论

五问答完裁决：`go-live ready | not ready`。五问任一缺失或空泛（无证据支撑）→ 直接 `not ready`；五问齐备且证据当次有效 → `ready`，并给出一句话理由。

## 证据纪律

- 先查再定：能从仓库、`rules/`、既有测试、当次 diff 查到的事实先查再定，不把提问当侦察手段；查不到的推断列入 `assumptions:` 清单，标注「结论基于假设」。
- 不调用 `question` 工具提问：需要用户裁决的事项写入问三卡点，由用户处置；绝不静默编造验收证据或给降级签收结论。
