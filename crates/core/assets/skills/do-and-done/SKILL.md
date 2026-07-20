---
name: do-and-done
description: Continuously drive the STATUS block (produced by task-plan) to go-live. Pick the next pending TODO, implement with bash/edit, verify with captured evidence, mark completed in the next STATUS block, then re-plan via task-plan and repeat. Only stop when progress is 100% and every go-live gate is green. Never finish incomplete — on a blocking question or irreversible operation, stop and report (do not proceed, do not wait if the question tool is unavailable).
---

# do-and-done —— 上线闭环执行

## 角色
执行契约。消费 task-plan 产出的 STATUS 块，把 TODO 一条条做到上线闭环，并在每轮 task-plan 刷新状态。不另起炉灶造清单。

> 本环境工具集为 `bash / edit / task`，**无 `todowrite` / `question` 工具**。状态在 STATUS 块中维护；暂停用「停下并上报」。

## 核心循环（每轮）
1. 读最近一次 task-plan 的 STATUS 块。
2. 取下一条 pending / in_progress TODO。
3. 实现（遵循全局规则：纯函数式、无 class、文件行数限制、无密钥）。
4. **验证并取证**：用 `bash` 跑 lint / typecheck / 测试 / 构建；记录命令与输出摘要作为证据。无证据不得标 completed。
5. 证据充分 → 在下一轮 task-plan 的 STATUS 块里标 completed；否则保持 in_progress 并记录原因。
6. 每完成一批（或单条关键项）→ 重新加载 task-plan，刷新 progress% 与 gate 状态。
7. 遇阻塞问题或不可逆操作（commit / push / DB 写 / 部署 / 迁移）→ 见下方「暂停协议」，**绝不自行越界执行**。
8. 长时间任务（≥120s）按全局规则后台执行 + 轮询，避免单调用超时。

## 暂停协议（阻塞 / 不可逆操作）
- `question` 工具可用（交互式 TUI）→ 调用暂停等人工；恢复后继续。
- 不可用（非交互 `run` / `opencode --loop`）→ **停下并上报**：清晰列出阻塞或待批操作、当前 progress、未决 TODO，结束本轮交还人工。
- 两种模式下都**绝不**未授权执行不可逆操作。

## 停止条件（唯一）
仅当同时满足才输出收尾：
- progress% = 100%（task-plan 判定）
- STATUS 块内 TODO 全 completed
- 所有 go-live gate 绿（或 N/A，见 task-plan 默认清单 / 仓库覆盖）

收尾时输出：`DONE / go-live ready`，附最终证据汇总与变更摘要。

## 永不半途而废
- 未达上线标准**绝不**输出完成、绝不退出。
- 阻塞 / 不可逆操作只允许「停下上报」（非交互）或「暂停等待」（交互），人工介入后继续推进。
- 触达 `steps` 上限仍没收尾 → 停下，输出当前 progress、未决项、阻塞点，交还人工，而非伪完成。

## 与 task-plan 的衔接
- do-and-done 消费并推进 task-plan 的 STATUS 块，不重复造清单。
- 范围 / 目标变化时立刻回到 task-plan 重新规划，再继续执行。

## 证据要求
- 每条 completed 必须可追溯：测试命令 + 结果、`file:line`、构建 / 日志摘要。
- 证据不足 → 退回 in_progress，不得计入 progress%。
