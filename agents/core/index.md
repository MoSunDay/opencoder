Commit: (working-tree, pre-initial-commit)

# core 模块

## 职责
跨 crate 共享的基础类型与配置。

## 关键抽象
- `Message`/`Role`/`ContentBlock`/`MessageUsage`（`src/message.rs`）：会话消息模型，serde 标签 `kind` snake_case。`Message::estimate_chars()` 遍历**所有** ContentBlock 变体（Text + Reasoning + ToolUse input JSON + ToolResult content）返回忠实文本渲染供 token 估算——区别于仅过滤 Text 的 `text()`（后者漏算 ToolResult/ToolUse/Reasoning，曾导致压缩从不触发）。
- `Config`（`src/config.rs`）：`provider/model/small_model/context_limit/max_tokens/reasoning_effort/interleaved_thinking/agent/compaction`，加 `providers: HashMap<String, ProviderConfig>`（命名 Provider 表：`base_url/api_key/model/headers`）+ `HttpHeader{name,value}`；`resolve_endpoint() -> Result<Endpoint>` 返回 env 解析后的端点（header value 支持 `{VAR}` 间接引用）；`provider_id()` 从 `model` 的 `"{provider}/{id}"` 前缀取活跃 provider 名。`interleaved_thinking: Option<bool>`（默认 `Some(true)`）——开启时 tool-call turn 的 `reasoning_content` 持久化到 assistant 消息并回传（交错思考，DeepSeek-V4 强制要求）。`load(workdir)` 三层合并（project `opencoder.json` / `.opencoder/config.json` → global）+ 环境变量覆盖（`OPENCODER_MODEL`/`OPENCODER_SMALL_MODEL`/`OPENCODER_CONTEXT_LIMIT`/`OPENAI_BASE_URL`）。`{VAR}` 形式 api_key 解析环境变量。`save(workdir, patch)`（项目优先、全局兜底）把 JSON merge-patch 写回 `opencoder.json`（深度合并，保留无关键），`save_target` 选首个含可编辑键的候选文件、无则在工作目录根创建 `opencoder.json`；`looks_like_env_var` 判定纯大写 `_` 串以决定 api_key 是否包成 `{NAME}` 引用。`network: NetworkConfig{proxy}`（LLM 流量代理源）。
- `Agent`/`AgentKind`/`AgentMode`/`ToolFilter` + 5 内置 agent（act/plan/explore/build/command）（`src/agent.rs`）。`AgentMode::{Primary,Subagent}` 区分主 agent 与子 agent；explore（只读，tools=search+read）/build（实现，tools=bash+edit）为 Subagent，act/plan/command 为 Primary。plan agent 工具 = bash + task（只读规划，bash 写命令被 bash_guard 拦截，build subagent 被 runner guard 拦截），不再有 plan_exit 工具——计划以纯文本输出，用户手动 Shift+Tab 切到 act 后自动开始执行。plan prompt（`base_prompt_plan`）通过 `.replace()` 从 BASE_PROMPT 剥离 `, 'build' (full tools) for implementation` 子句，使模型在 plan 模式下不知道 build subagent 存在；act prompt 保留完整 BASE_PROMPT。
- `Tool` trait / `ToolArc` / `ToolContext` / `ToolOutput`（`src/tool.rs`）。
- `Skill`（`src/skill.rs`）：用户可编排的「技能」指令包（`name/description/body/source`；其中 `source` 为磁盘路径，经纯函数 `body_with_source(&Skill)` 以 `> Source: <path>` 前缀注入 body——session skill_resolve / TUI app_helpers / CLI run 在激活 skill 时调用，使 agent 能定位 skill 同目录资产（如 EXAMPLES.md））。`skills_dir()` 返回 `~/.opencoder/skills`（二进制自有配置主目录，与 config 同源）；`discover()` 扫描该目录，识别 `<name>.md` 与 `<name>/SKILL.md` 两种布局，解析可选 `---` YAML frontmatter（`name`/`description`，缺省回退文件名/首行）。目录缺失返回空 `Vec`（非错误）。`extract_skill_tokens(text)` 剥离**所有** `$name` token（仅用于发现/激活）；`strip_resolved_skill_tokens(text, resolved)` 只剥离已解析 token、unresolved `$name` 原样保留（杜绝 token 吞吃用户输入内容），由 TUI/runner 解析器在 resolve 后重建 clean 文本。二进制经 `include_str!` 内嵌并随附内置 skill 包（seed 表与 seeding/install 逻辑在 `src/skill/seed.rs`，发现/解析/token 处理留在 `src/skill.rs`），首启 seed 到 `~/.opencoder/skills`：`task-plan`（全局规划——产出 STATUS 块，拆解 TODO + 验收方案 + 依赖影响分析，是工作流起点）、`do-and-done`（实现/执行循环）、`repo-local-memory`（仓库本地记忆）、`review`（上线前只读评审）、`summary`（任务回顾/recap——在任一节点 done/paused/handoff 产出结构化回顾：需求/实际变更/验证证据/优化空间，只读不修改）、`submit`（提交/PR）；`task-plan → do-and-done → review → submit` 为主链，`summary` 为正交的任务回顾工具（任一节点可用，不强制插入主链），`say-and-replay` 为与之并列的正交工具。另设 `DEP_GATED_SKILLS` 桶（现含 `ssh-pty`、`chrome-headless`）：经 `.skills-deps` sentinel 门控的 opt-in 技能，与内置 seeding 独立——用户运行 `install-skills-dep.sh`（首启经 `write_install_script` idempotent 落盘到 `~/.opencoder/`）创建 sentinel 后下次启动 seed；`chrome-headless` **不打包浏览器二进制**，指导 agent 用 bash 工具驱动本地 Chrome（`--headless=new --dump-dom`，API 优先）。两者 seeding 均 per-file 增量、never-clobber（用户改动永久存活、二进制升级时缺文件仍补写）。
- `CompactionConfig`（`src/config.rs`）：`auto/context_threshold/tail_turns/reserved/buffer`（`prune` 字段已移除——曾为死配置）。
- `OutputStreamlineConfig`（`src/config.rs`）：`enabled/trim_trailing/collapse_blank_lines/trim_outer`（默认全开）+ `collapse_inline_ws`（默认关，opt-in）。session 在 `run_loop` 持久化前对完成的 assistant 文本做保义精简（见 session 模块）。
- `net` 模块（`src/net.rs`）：`build_http_client`/`build_http_client_with_read_timeout`/`effective_proxy`——proxy-aware reqwest 客户端，**loopback bypass**（`127.0.0.1`/`localhost`/`::1`/`0.0.0.0` 永不经代理，否则本地 mock/自连在代理环境下被截断）。被 llm client 使用。
- `data_dir` 模块（`src/data_dir.rs`）：`data_dir_for(workdir: &Path) -> PathBuf`——唯一的 per-workdir 数据目录解析（`<data_local>/opencoder/<hash>`，hash 为 DefaultHasher over workdir 规范化字符串形式，先 canonicalize 故 `/p` 与 `/p/` 及 symlink 折叠为同一目录）。替代此前 cli/web/tui 三处各自漂移的副本，三进程对同一 workdir 解析出同一 data dir，使 session 跨进程可见。同模块另暴露 `data_root() -> PathBuf`（=`<data_local>/opencoder`，`data_dir_for` 即 `data_root().join(hash)`），供 `ts -l` 等全局操作扫描**所有** workdir 的 store。二者均经 `lib.rs` re-export。

## 主流程
Config::load 顺序：默认 → 全部已存在候选**深度合并**（global base → project override，project 后写后赢）→ env 覆盖。候选顺序（从最具体到最全局）：`<workdir>/.opencoder/config.json`、`<workdir>/opencoder.json`、`~/.opencoder/config.json`、`~/.opencoder/opencoder.json`、`~/.config/opencoder/config.json`。这样 `~/.opencoder` 提供 provider+key 作为基底，项目 opencoder.json 仅覆盖 model 等字段——`opencoder` 从任意目录直接执行。

## 依赖与接口
- 依赖：serde、chrono、dirs、async-trait、reqwest（`net` 模块构造 proxy-aware 客户端，含 `socks` feature）。
- `net`（`build_http_client`/`effective_proxy`）关键项经 `lib.rs` re-export，供 llm/session 直接调用。`data_dir_for` 同样经 `lib.rs` re-export，供 cli/tui/web 解析同一 data dir 后打开 store。
- 被依赖：所有其它 crate（类型来源）。

## 相关模块
- [agents/session](../session/index.md) — Config 驱动压缩与模型选择。
- [agents/llm](../llm/index.md) — Message lowering。
