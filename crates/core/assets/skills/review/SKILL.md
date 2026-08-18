---
name: review
description: five mandatory questions — (1) restate the original goal, (2) replay what was done with completed/total+percent, (3) list blockers resolved and open, (4) show per-item verification with fresh evidence (re-runs gates itself, guards against green-washing, checks regression baseline, test quality/layering/structural coverage, blast radius), (5) name next TODOs — then rules go-live readiness from those answers. No fixed output template; answering the five questions well IS the output.
---

# review —— 五问式上线前评审

## 问一：原始需求目标是什么？
- 取自用户原始 prompt：逐字或贴近复述目标与验收标准，**只复述不判定**。
- 从执行路径评估是否偏航。

## 问二：做了哪些事情？做到了多少？
- 逐条回放完成点：每条必须带 **verify（怎么验证的）+ evidence（当次证据：输出尾段 / file:line）**，缺一不计入完成。
- 完成度 = completed/total + 百分比（向下取整），仅计入证据俱全的条目。

## 问三：过程中遇到了什么卡点？
- 已解除的卡点 + 解除方式；仍 open 的标 pending。
- 无卡点也要明说 none，不许跳过。

## 问四：每个完成点怎么验证的？证据是什么？
**没有证据 = 没有通过**，且证据必须是**当次实跑**——STATUS 块的旧摘要、上一会话的输出都不算。逐项核查：

## 问五：下一步 TODO 是什么？
- 结论 ready → 后续建议，或明说 none。
- 结论 not ready → 与缺口一一对应的修复 TODO，各附验收标准。

## 上线结论
五问答完后裁决 `go-live ready | not ready`：
- **五问任一缺失或空泛 → 直接 not ready**（等价于证据不充分）。
