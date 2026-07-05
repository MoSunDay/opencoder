Commit: (working-tree, pre-initial-commit)

# OpenCoder 能力地图

OpenCoder 当前提供以下用户/调用方可感知的能力。每项链接到对应的业务文档或逻辑模块。

## 能力组

- **会话恢复**：CLI `--session <id>` / `--continue` / `--fork`，跨进程从 libsql 重建历史；title 由 small_model 异步生成。详见 [agents/session](../agents/session/index.md)。
- **模型与压缩配置化**：`provider/model`、`small_model`、`context_limit`、`max_tokens`、`compaction.{auto,context_threshold,reserved,tail_turns,prune,buffer}` 全可经 opencode.json / 环境变量 / CLI flag 配置；压缩首轮即可由 token 估算触发。详见 [agents/session](../agents/session/index.md)、[agents/core](../agents/core/index.md)。
- **steer / followup（两段式 delivery）**：运行中的会话可在 turn 边界吸收 steer（重置步数配额），idle 时消费恰好一条 queue。HTTP `POST /prompt` 带 `delivery`。详见 [agents/session](../agents/session/index.md)。
- **Web 会话操作**：HTTP CRUD + SSE 事件流（replay+live）+ 运行时 agent/model 切换 + interrupt。详见 [agents/web](../agents/web/index.md)。
- **高性能本地存储**：libsql 嵌入 + WAL，并发读写；`Store` trait 为切换其它 Rust SQLite 实现留口子。详见 [agents/store](../agents/store/index.md)。
- **glm5.2 端到端**：`scripts/e2e-glm.sh` 用真 glm5.2 写贪吃蛇/雷霆战机，验证工具链/多轮/恢复/压缩/web 全链路。
- **TUI**：3-region 布局（body / composer / 底部合并 status：model + agent + dir + ctx%）+ ratatui Scrollbar + 自动跟随（跟随态显示「跟随中…」，上滚后变可点击 ↓ 跳底，已启用鼠标捕获）+ 运行中 braille spinner 动画 + 可见 bar 光标 + 实时上下文百分比 + subagent 渲染 + steer/followup（Ctrl+O / Ctrl+J）+ 双击 Esc 硬中止。快捷键：Ctrl+T 切 plan/act（按当前模式双向）、Ctrl+D 退出、Esc 关 help。详见 [迭代三](changelog/2026-07-05/iteration3-tui-overhaul.md) / [迭代四](changelog/2026-07-05/iteration4-tui-ux-hardabort.md)。
- **Skill 选择（TUI `$`）**：空 composer 输入 `$` 弹出技能选择器，扫描 `~/.opencoder/skills`（识别 `<name>.md` 与 `<name>/SKILL.md`，可选 `---` frontmatter 的 `name`/`description`）。↑/↓ 移动、键入即过滤、Enter 激活、Esc 取消。激活后 skill 正文以 `## Active skill` 段注入系统提示末尾（最高优先级），整会话生效，状态栏显示 `skill:<name>`；再次选择替换，已激活时首行「✕ clear」清除。详见 [agents/core](../agents/core/index.md)、[agents/session](../agents/session/index.md) / [skill-picker changelog](changelog/2026-07-05/skill-picker.md)。
- **测试规则与覆盖**：[rules/](../rules/) 目录建立强制测试规则（每功能必有测试 + 迭代回归 gate + 测试分层）；workspace 测试 144 个，覆盖 LLM 流式原语 / CLI / 工具 / subagent / TUI / Web / prompt。详见 [迭代四 changelog](changelog/2026-07-05/iteration4-test-coverage-rules.md)。

## changelog 入口
- [changelog 根目录](changelog/) —— 按日期记录的可检索变更主题。
