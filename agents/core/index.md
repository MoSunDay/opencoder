Commit: (working-tree, pre-initial-commit)

# core 模块

## 职责
跨 crate 共享的基础类型与配置。

## 关键抽象
- `Message`/`Role`/`ContentBlock`/`MessageUsage`（`src/message.rs`）：会话消息模型，serde 标签 `kind` snake_case。
- `Config`（`src/config.rs`）：`provider/model/small_model/context_limit/max_tokens/reasoning_effort/agent/compaction`。`load(workdir)` 三层合并（project `opencode.json` / `.opencode/config.json` → global）+ 环境变量覆盖（`OPENCODE_MODEL`/`OPENCODE_SMALL_MODEL`/`OPENCODE_CONTEXT_LIMIT`/`OPENAI_BASE_URL`）。`{VAR}` 形式 api_key 解析环境变量。`save(workdir, patch)`（项目优先、全局兜底）把 JSON merge-patch 写回 `opencode.json`（深度合并，保留无关键），`save_target` 选首个含可编辑键的候选文件、无则在工作目录根创建 `opencode.json`；`looks_like_env_var` 判定纯大写 `_` 串以决定 api_key 是否包成 `{NAME}` 引用。
- `Agent`/`AgentKind`/`AgentMode`/`ToolFilter` + 5 内置 agent（act/plan/explore/build/command）（`src/agent.rs`）。`AgentMode::{Primary,Subagent}` 区分主 agent 与子 agent；explore（只读）/build（全工具）为 Subagent，act/plan/command 为 Primary。
- `Tool` trait / `ToolArc` / `ToolContext` / `ToolOutput`（`src/tool.rs`）。
- `Skill`（`src/skill.rs`）：用户可编排的「技能」指令包（`name/description/body/source`）。`skills_dir()` 返回 `~/.opencoder/skills`（二进制自有配置主目录，与 config 同源）；`discover()` 扫描该目录，识别 `<name>.md` 与 `<name>/SKILL.md` 两种布局，解析可选 `---` YAML frontmatter（`name`/`description`，缺省回退文件名/首行）。目录缺失返回空 `Vec`（非错误）。
- `CompactionConfig`（`src/config.rs`）：`auto/context_threshold/tail_turns/reserved/prune/buffer`。

## 主流程
Config::load 顺序：默认 → 全部已存在候选**深度合并**（global base → project override，project 后写后赢）→ env 覆盖。候选顺序（从最具体到最全局）：`<workdir>/.opencode/config.json`、`<workdir>/opencode.json`、`~/.opencoder/config.json`、`~/.opencoder/opencode.json`、`~/.opencode/config.json`、`~/.config/opencode/config.json`。这样 `~/.opencoder` 提供 provider+key 作为基底，项目 opencode.json 仅覆盖 model 等字段——`opencoder` 从任意目录直接执行。

## 依赖与接口
- 依赖：serde、chrono、dirs、async-trait。
- 被依赖：所有其它 crate（类型来源）。

## 相关模块
- [agents/session](../session/index.md) — Config 驱动压缩与模型选择。
- [agents/llm](../llm/index.md) — Message lowering。
