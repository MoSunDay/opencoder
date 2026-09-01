Commit: (working-tree, sandbox 回退为 plan：恢复 plan/act 双模式回切，写拦截能力保留)

# sandbox 回退为 plan：恢复 plan/act 双模式回切（写拦截能力保留）

## 背景

64a4878（2026-08-28，[sandbox 模式替换 plan/act 双模式](../2026-08-28/sandbox-mode-replace-plan-act.md)）把只读模式更名为 `sandbox` 并删除了 `/plan` 拼写。本次按需求把 sandbox 整体改回 plan：模式名、命令拼写、提示词、守卫文案、chip 与 API 值全部回到 `plan` 语义；plan ⇄ act 回切保持纯状态切换（持久化 + `AgentSwitch`，不折叠）。**运行逻辑零回退**——不恢复 plan_phase/plan_handoff 机械，只保留现有计划执行功能；bash 写拦截（bash_guard）能力原样保留，仅文案 plan 化。

## 实现

- **core**：`AgentKind::Sandbox` → `AgentKind::Plan`（serde `alias = "sandbox"` 兼容 interlude 存量 payload）；builtin `plan`（Primary，tools=bash/task/**question 注入**），`base_prompt_plan` 只读约束 + `question` 澄清指引；build subagent 保持 bash+edit、**不注入 question**（`question_tool_is_plan_and_act_only` 结构守卫）。
- **session**：`/plan` ⇄ `/act` 控制命令（`/sandbox` 由 CLI legacy 前缀重写承接）；bash 写拦截文案 plan 化（"Blocked in plan mode … To make changes, switch to the act agent"）；prompt 注入 `IN_PLAN_MODE`；plan 恒可见 `question`（latent 豁免保留）；explore-only 子代理与 task schema 隐 build 不变。
- **task-plan skill 去 question 描述注入**：澄清协议保留，删除「plan 模式常驻可见，act 模式由本 skill 解锁」的工具可见性描述（`question` 仍在 body 前 500 字符解锁窗口内）。
- **store**：读路径 `normalize_agent` 反转——interlude 存量 `agent='sandbox'` 行归一为 `plan`（原始行不重写）；`AgentKind` serde alias 双保险。
- **cli**：`--agent plan` 恢复合法；`--agent sandbox` 解析期报错提示 renamed back to 'plan'；`rewrite_legacy_sandbox_prefix` 把前导 `/sandbox` 重写为 `/plan`。
- **tui/web**：`/plan` 命令面板、chip `[plan]`（warn 橙）、`post_agent` 接受 act|plan；SPA `agent_switched` 值泛化不变。
- **测试**：store/会话 legacy resume 测试反转为 sandbox→plan；`parse_rejects_removed_sandbox_command`、`agent_flag_rejects_removed_sandbox_agent_with_rename_hint`、`legacy_sandbox_prefix_rewrites_to_plan` 等 pin 新契约。

## 有意保留

- `sandbox` 仅作为 legacy 拼写存在于：store 读路径归一化、`AgentKind` serde alias、CLI 前缀重写与 `--agent` 报错提示。新代码一律 `plan`。
- Chromium 启动参数 `--no-sandbox` 与 README 的「no sandbox」（OS 级沙箱）同形不同义，不属本模式，未改动。
