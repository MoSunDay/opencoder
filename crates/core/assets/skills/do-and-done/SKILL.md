---
name: do-and-done
description: Continuously drive the launch-closure plan produced by task-plan to go-live. Pick the next dependency-ready item, implement it, verify with captured evidence, refresh the plan as facts change, and repeat until the original goal, critical path, production-equivalent validation, and release conditions are fully closed with fresh evidence. Never finish incomplete — on a blocking question or irreversible operation, stop and report.
---

# do-and-done —— 上线闭环执行

## 角色
执行契约。消费 task-plan 产出的闭环计划，按依赖顺序把计划项逐条做到可上线，并在事实、范围或证据变化时重新规划。不另起炉灶造清单。

> 使用当前环境实际提供的工具推进；缺少交互式提问能力时，阻塞按「停下并上报」处理，不得猜测或绕过。

## 核心循环（每轮）
1. 读最近一次 task-plan 的问题范围、闭环计划、依赖 / blocker、验收方式和上线关键路径。
2. 取下一条依赖已满足、可立即执行的计划项；优先 P0/P1 和关键路径。
3. 实现（遵循全局规则：纯函数式、无 class、文件行数限制、无密钥）。
4. **验证并取证**：用 `bash` 跑 lint / typecheck / 测试 / 构建；记录命令与输出摘要作为证据。无证据不得标 completed。
5. 证据充分才把计划项视为完成；否则明确记录证据缺口、失败原因和下一动作。
6. 每完成一批，或出现新事实 / 新风险 / 范围变化时，重新加载 task-plan 刷新缺口、依赖、验证与上线路径。
7. 遇阻塞问题或不可逆操作（commit / push / DB 写 / 部署 / 迁移）→ 见下方「暂停协议」，**绝不自行越界执行**。
8. 长时间任务（≥120s）按全局规则后台执行 + 轮询，避免单调用超时。

## 暂停协议（阻塞 / 不可逆操作）
- `question` 工具可用（交互式 TUI）→ 调用暂停等人工；恢复后继续。
- 不可用（非交互 `run` / `opencode --loop`）→ **停下并上报**：清晰列出阻塞或待批操作、当前完成度、未决计划项，结束本轮交还人工。
- 两种模式下都**绝不**未授权执行不可逆操作。

## 停止条件（唯一）
仅当同时满足才输出收尾：
- 原始目标和关键路径内计划项全部有验收证据
- 线上 / 生产等价验证已完成，或有充分的不适用说明
- 无未闭环 blocker，发布、观测与回滚条件成立
- 以当次新鲜证据裁决达到 `go-live ready`，不凭历史结论签收

收尾时输出：`DONE / go-live ready`，附最终证据汇总与变更摘要。

## 永不半途而废
- 未达上线标准**绝不**输出完成、绝不退出。
- 阻塞 / 不可逆操作只允许「停下上报」（非交互）或「暂停等待」（交互），人工介入后继续推进。
- 触达 `steps` 上限仍没收尾 → 停下，输出当前完成度、未决项、阻塞点，交还人工，而非伪完成。

## 与 task-plan 的衔接
- do-and-done 消费并推进 task-plan 的闭环计划，不重复造清单。
- 范围 / 目标变化时立刻回到 task-plan 重新规划，再继续执行。

## 证据要求
- 每条 completed 必须可追溯：测试命令 + 结果、`file:line`、构建 / 日志摘要。
- 证据不足 → 保持未完成，不得计入需求完成百分比。
