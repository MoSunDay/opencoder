Commit: (working-tree)

# cli 模块

## 职责
clap 命令前端 + headless 运行时。解析全局 flag 与子命令（run/tui/serve/config/models/session），把用户意图分发到 session/web/store 层。headless 模式（`run` 或裸 prompt）是 e2e 与脚本化的主入口。

## 边界与非目标
- 不做终端渲染（TUI 在 `opencode_tui`）、不做 HTTP 服务实现（web 在 `opencode_web`）。
- 不持有长期运行态——headless 一次性 run 完即退；serve 委托 web。
- 非目标：CLI 不直接暴露 steer/queue 两段式 delivery（那是 web `POST /prompt` 的 `delivery` 字段）；CLI headless 单 prompt。

## 关键抽象
- `Cli`（`src/lib.rs`）：全局 flag `--model/--small-model/--agent/--workdir/--session/--continue/--fork/--verbose/--serve` + `Command::{Run, Tui, Serve, Config, Models, Session}` + trailing `prompt`。
- `SessionSub::{List, Show{id, json}, Delete, Export{id, out}, Import{input}}`（`src/lib.rs`）。`Show --json` 是深度观测面（见下）。
- `run_headless`（`src/run.rs`）：建/恢复 SessionState → `run(session, prompt, print_event)` → 异步 `generate_title`。`--continue` 取最新 session；`--session <id>` 指定；`--fork` 在 resume 前调 `fork_session` 复制（原 session 零修改）。
- `fork_session(store, parent_id)`（`src/run.rs`）：读 parent meta+messages → 新 id → `create_session` + `append_messages`，返回新 id，打印 `[forked P → C]`。
- `print_event`（`src/run.rs`）：headless 事件渲染——`▸ name input`（ToolStart，input 取 command/path/description 摘要）、缩进输出（ToolEnd，错误红色）、`[context compacted] summary`、`⤷ subagent [kind] prompt` / `✔|✘ summary`、`[switched to X mode]`、`[session <id>]`（run 结束）、`[status]`。`ReasoningDelta`/`TranscriptReset`/`QueueConsumed`/`SubagentChild` 不打印。这套 marker 是 e2e 日志断言的稳定来源。
- `build_session_json(store, id)`（`src/session_cmd.rs`）：返回 `{meta, messages, subagent_tasks}` 的 JSON 值——meta 含 compaction summary 字段；messages 含**全部** ContentBlock（Text/Reasoning/ToolUse/ToolResult，不过滤）；subagent_tasks 含 status/result/ok。`session show <id> --json` 打印之。这是 e2e 深度断言的机器可读观测面，解耦存储内部（不依赖 sqlite 直查 / hash 路径）。
- `data_dir_for(workdir)`（`src/session_cmd.rs`）：workdir → 本地数据目录（`<data_local>/opencode/<hash>/opencode.db`）。**注意**：仍用 `std::collections::hash_map::DefaultHasher`（std 不保证跨版本稳定）——web 层已在 go-live review 改用 FNV-1a，CLI 层尚未同步，是已知隐患。

## 主流程
- 裸 prompt / `run`：`run_headless` → 一次性 run → `[session <id>]`。
- `--continue`：`pick_resume_id` 取 `list_sessions(limit=1)` 最新 → resume。
- `--session <id> [--fork]`：resume 指定 id；`--fork` 先复制。
- `session show <id> [--json]`：默认按 `[role] text()` 打印（仅 Text 块）；`--json` 打印完整状态。
- `session export <id> -o <file>` / `session import <file>`：见 [agents/store](../store/index.md) 的 bundle。

## 依赖与接口
- 依赖：clap、opencode-core、opencode-llm（ChatClient）、opencode-session（run/resume/generate_title）、opencode-store、opencode-web（serve）。
- 被依赖：binary crate（`src/main.rs` 解析 `Cli` 并分发）。

## 相关模块
- [agents/session](../session/index.md) — headless run/resume/fork 的核心。
- [agents/store](../store/index.md) — session 子命令 + bundle 导出导入。
- [agents/web](../web/index.md) — serve 子命令。

## 代表性锚点
- 深度观测面测试：`session_cmd::tests::build_session_json_emits_meta_messages_and_subagent_tasks` / `build_session_json_errors_on_missing_session`
- fork 实现测试：`cli/tests/fork_session.rs`
- CLI 解析测试：`cli/tests/cli_parse.rs`（含 `session show --json` 解析）
- headless 事件渲染：`run::tests::{summarize_input_extracts_command, truncate_adds_ellipsis}`
