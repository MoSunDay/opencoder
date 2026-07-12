Commit: (working-tree, pre-initial-commit)

# core 模块

## 职责
跨 crate 共享的基础类型与配置。

## 关键抽象
- `Message`/`Role`/`ContentBlock`/`MessageUsage`（`src/message.rs`）：会话消息模型，serde 标签 `kind` snake_case。`Message::estimate_chars()` 遍历**所有** ContentBlock 变体（Text + Reasoning + ToolUse input JSON + ToolResult content）返回忠实文本渲染供 token 估算——区别于仅过滤 Text 的 `text()`（后者漏算 ToolResult/ToolUse/Reasoning，曾导致压缩从不触发）。
- `Config`（`src/config.rs`）：`provider/model/small_model/context_limit/max_tokens/reasoning_effort/interleaved_thinking/agent/compaction`。`interleaved_thinking: Option<bool>`（默认 `Some(true)`）——开启时 tool-call turn 的 `reasoning_content` 持久化到 assistant 消息并回传（交错思考，DeepSeek-V4 强制要求）。`load(workdir)` 三层合并（project `opencoder.json` / `.opencoder/config.json` → global）+ 环境变量覆盖（`OPENCODER_MODEL`/`OPENCODER_SMALL_MODEL`/`OPENCODER_CONTEXT_LIMIT`/`OPENAI_BASE_URL`）。`{VAR}` 形式 api_key 解析环境变量。`save(workdir, patch)`（项目优先、全局兜底）把 JSON merge-patch 写回 `opencoder.json`（深度合并，保留无关键），`save_target` 选首个含可编辑键的候选文件、无则在工作目录根创建 `opencoder.json`；`looks_like_env_var` 判定纯大写 `_` 串以决定 api_key 是否包成 `{NAME}` 引用。
- `Agent`/`AgentKind`/`AgentMode`/`ToolFilter` + 5 内置 agent（act/plan/explore/build/command）（`src/agent.rs`）。`AgentMode::{Primary,Subagent}` 区分主 agent 与子 agent；explore（只读）/build（全工具）为 Subagent，act/plan/command 为 Primary。plan agent 工具 = bash + task（只读规划，bash 写命令被 bash_guard 拦截，build subagent 被 runner guard 拦截），不再有 plan_exit 工具——计划以纯文本输出，用户手动 Shift+Tab 切到 act 后自动开始执行。plan prompt（`base_prompt_plan`）通过 `.replace()` 从 BASE_PROMPT 剥离 `, 'build' (full tools) for implementation` 子句，使模型在 plan 模式下不知道 build subagent 存在；act prompt 保留完整 BASE_PROMPT。
- `Tool` trait / `ToolArc` / `ToolContext` / `ToolOutput`（`src/tool.rs`）。
- `Skill`（`src/skill.rs`）：用户可编排的「技能」指令包（`name/description/body/source`）。`skills_dir()` 返回 `~/.opencoder/skills`（二进制自有配置主目录，与 config 同源）；`discover()` 扫描该目录，识别 `<name>.md` 与 `<name>/SKILL.md` 两种布局，解析可选 `---` YAML frontmatter（`name`/`description`，缺省回退文件名/首行）。目录缺失返回空 `Vec`（非错误）。
- `CompactionConfig`（`src/config.rs`）：`auto/context_threshold/tail_turns/reserved/buffer`（`prune` 字段已移除——曾为死配置）。

## 主流程
Config::load 顺序：默认 → 全部已存在候选**深度合并**（global base → project override，project 后写后赢）→ env 覆盖。候选顺序（从最具体到最全局）：`<workdir>/.opencoder/config.json`、`<workdir>/opencoder.json`、`~/.opencoder/config.json`、`~/.opencoder/opencoder.json`、`~/.config/opencoder/config.json`。这样 `~/.opencoder` 提供 provider+key 作为基底，项目 opencoder.json 仅覆盖 model 等字段——`opencoder` 从任意目录直接执行。

## 依赖与接口
- 依赖：serde、chrono、dirs、async-trait。
- 被依赖：所有其它 crate（类型来源）。

## 相关模块
- [agents/session](../session/index.md) — Config 驱动压缩与模型选择。
- [agents/llm](../llm/index.md) — Message lowering。
