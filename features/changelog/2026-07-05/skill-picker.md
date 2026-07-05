Commit: (working-tree, pre-initial-commit)

# Skill 选择器（TUI `$`）：从 ~/.opencoder/skills 加载技能并注入系统提示

## Context
OpenCoder 此前的 agent 身份（act/plan/subagent/command）为编译期硬编码（`crates/core/src/agent.rs::builtin_agents`），无运行时可加载的「技能」概念；`agents/session/index.md` 亦将 skills 列为非目标。用户希望像 opencode 那样把可复用的操作规程（一个 SKILL.md 一个主题）放进 `~/.opencoder/skills/`，并在 TUI 里随时挂到当前会话上指导 agent，而不必改 agent 注册表或配置。

## Change Summary
- **core：skill 模块**（`crates/core/src/skill.rs`，新增）：`Skill{name,description,body,source}`；`skills_dir()` 返回 `~/.opencoder/skills`（二进制自有配置主目录，与 config 同源）；`discover()` 扫描该目录，识别 `<name>.md` 与 `<name>/SKILL.md` 两种布局，解析可选 `---` YAML frontmatter（`name`/`description`，缺省回退文件名 stem / 首行非标题行），按 name 排序。目录缺失或无 `.md` 返回空 `Vec`（非错误）。从 `lib.rs` 导出 `Skill`/`discover_skills`/`skills_dir`。
- **session：skill_prompt 注入**：`SessionState` 增 `skill_prompt: Option<String>` + `with_skill` builder；`build_system(agent, working_dir, skill_prompt)` 在环境块之后追加 `## Active skill` 段（系统提示末尾 = 最高优先级）。runner/compaction 两处调用点已接通。
- **tui：`$` 选择器**（`crates/tui/src/menu.rs`，新增）：`SkillMenu` 状态机 + `handle_menu_key`（↑/↓ 移动、键入过滤、Backspace、Enter 确认、Esc 取消、Ctrl+C 仍可退出）；`render_skill_popup` 居中浮层（ratatui `List`/`ListState`，复用 help 浮层的 `Clear`+居中 `Rect` 模式）+ filter footer。已激活时首行「✕ clear」清除。
- **tui：app.rs 接线**：`$` 且 composer 为空时打开选择器（模态拦截按键）；新增 `UiCmd::SetSkill(Option<String>)` / `KeyAction::SetSkill(Option<(name,body)>)`；worker 设置 `sess.skill_prompt`；状态栏显示 `skill:<name>`；help（`keybind.rs`）加 `$ select skill`。

## Impact Surface
- 新增文件：`crates/core/src/skill.rs`（267 行）、`crates/tui/src/menu.rs`（377 行）、`crates/core/tests/skill_contract.rs`（98 行，7 测试）。
- 修改：`crates/core/src/lib.rs`（导出 skill）、`crates/session/src/lib.rs`（skill_prompt 字段 + with_skill）、`crates/session/src/prompt.rs`（build_system 签名 +1 参数）、`crates/session/src/{runner,compaction,resume}.rs`（调用点 / 字段初始化）、`crates/tui/src/{app.rs,lib.rs,keybind.rs}`。
- 行为契约：skill 选择为「整会话黏贴」——激活后每轮 `build_system` 都注入，直至换选或 clear；非 one-shot。
- 文档：`agents/core`、`agents/session`（修正「skills 非目标」的过时陈述）、`features/index.md`。

## Notes / Compatibility
- 触发条件：仅当 composer 为空且光标在行首时输入 `$` 才开浮层；`$5` 等字面量照常插入。
- 目录扫描同步阻塞但极小（单层 read_dir），在按键处理中直接调用。
- skill 仅影响系统提示；不改 agent 的工具集 / kind / mode。
- `app.rs` 因累积特性（鼠标/scrollbar/spinner/硬中止等）已达 853 行（迭代上限 800）；本特性的按键逻辑已外置到 `menu.rs`，app 内仅余触发、match arm 与渲染接线约 30 行。

## Related Docs
- [agents/core](../../../agents/core/index.md)
- [agents/session](../../../agents/session/index.md)
