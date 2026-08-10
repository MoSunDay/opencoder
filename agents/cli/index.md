Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# cli 模块

## 职责
clap 命令前端 + headless 运行时。解析全局 flag 与子命令（run/tui/ts/server/client/config/models/session/update，`ts` 别名 `rs`），把用户意图分发到 session/web/store 层。headless 模式（`run` 或裸 prompt）是 e2e 与脚本化的主入口。

## 边界与非目标
- 不做终端渲染（TUI 在 `opencoder_tui`）、不做 HTTP 服务实现（web 在 `opencoder_web`）。
- 不持有长期运行态——headless 一次性 run 完即退；serve 委托 web。
- 非目标：CLI 不直接暴露 steer/queue 两段式 delivery（那是 web `POST /prompt` 的 `delivery` 字段）；CLI headless 单 prompt。

## 关键抽象
- `Cli`（`src/lib.rs`）：全局 flag `--model/--image/--workdir/--session/--continue/--fork/--verbose/--prompt-file` + `Command::{Run, Tui, Ts, Server, Client, Config, Models, Session, Update}`（`Update` 无参，用内置提示词经 `run_headless` 委托代理执行自更新：clone latest main → build → 原子替换 PATH 二进制）（`Ts` 别名 `rs`；裸 `ts`/`rs` 在 tmux 内外都恒新建 `opencode-<ulid>` tmux session，tmux 内用 detached-create + `switch-client`，不再退化为 inline TUI。`registry.rs` 在每个 per-workdir store 旁原子记录 canonical `workdir` marker，并分页扫描全局 registry；`-l` 展示 live 与 stopped 的真实 workdir及 8 字符 ID 前缀，marker 图例 `*`=attached、`·`=live(detached)、`-`=stopped，ts 归属谓词 `is_ts_owned` 以持久标记 `model IS NULL` 判定（mode-switched ts 会话 `persist_agent` 只 patch `agent`、`model` 保持 NULL，仍计入 stopped；普通 `tui`/`run` 首条消息必落库 model，故被排除；`ts_start` 现在只调 `record_workdir`（仅写 workdir marker），不再 `ensure_session` 预置 DB session 行——该行改为由首条 `record()` 经 `ts_origin` flag 惰性创建，`model IS NULL` 标记因此仍由 `ts_origin` 在首次 persist 时写入而非预置行），先按 workdir 升序分组、组内按创建时间倒序，同一时间以完整 ID 稳定排序；`-r <id>` / `-d <id>` 都会把该前缀在 live tmux 与全部 store 中解析为唯一完整 ID，歧义时拒绝操作；`-r` 从任意目录按记录路径重连/冷启动，`-c` 批量清理所有停止的 ts-owned 行，`-d` 对非当前 live tmux 先 kill 再删唯一 Store 记录，当前 session 与重复 ID 拒绝删除；普通 `tui`/`run` 历史不在管理范围）+ trailing `prompt`。
- `SessionSub::{List, Show{id, json}, Delete, Export{id, out}, Import{input}}`（`src/lib.rs`）。`Show --json` 是深度观测面（见下）。
- `ConfigSub::{Show, Set{model}}`（`src/lib.rs`）。`Config Show` 输出合并后配置 JSON；`Config Set <model>` 经 `config_dispatch` → `Config::save` 把 `provider/model_id` 写回 opencoder.json（设全局默认模型的脚本化入口；与 TUI `/model` 的 `y=global` / Web `POST /model persist_default=true` 语义一致）。
- `run_headless`（`src/run.rs`）：建/恢复 SessionState → `run(session, prompt, print_event)` → 异步 `generate_title`。`--continue` 取最新 session；`--session <id>` 指定；`--fork` 在 resume 前调 `fork_session` 复制（原 session 零修改）。**resume 摘要**：resume 后调 `print_resume_summary(&session).await`，蓝字单行 `⤷ resumed session: done/total subagents done — ✔explore … ✘build …`（空→不打印）；格式逻辑抽出为纯函数 `pub(crate) fn format_resume_summary(&[SubagentTaskRecord]) -> Option<String>` 便于单测。
- `fork_session(store, parent_id)`（`src/run.rs`）：读 parent meta+messages → 新 id → `create_session` + `append_messages`，返回新 id，打印 `[forked P → C]`。
- `print_event`（`src/run.rs`）：headless 事件渲染——`▸ name input`（ToolStart，input 取 command/path/description 摘要）、缩进输出（ToolEnd，错误红色）、`[context compacted] summary`、`⤷ subagent [kind] prompt` / `✔|✘ summary`、`[switched to X mode]`、`[session <id>]`（run 结束）、`[status]`。`ReasoningDelta`/`TranscriptReset`/`QueueConsumed`/`SubagentChild` 不打印。这套 marker 是 e2e 日志断言的稳定来源。
- `build_session_json(store, id)`（`src/session_cmd.rs`）：返回 `{meta, messages, subagent_tasks}` 的 JSON 值——meta 含 compaction summary 字段；messages 含**全部** ContentBlock（Text/Reasoning/ToolUse/ToolResult，不过滤）；subagent_tasks 含 status/result/ok。`session show <id> --json` 打印之。这是 e2e 深度断言的机器可读观测面，解耦存储内部（不依赖 sqlite 直查 / hash 路径）。
- `data_dir_for(workdir)`（`src/session_cmd.rs`）：workdir → 本地数据目录（`<data_local>/opencoder/<hash>/opencoder.db`）。**注意**：仍用 `std::collections::hash_map::DefaultHasher`（std 不保证跨版本稳定）——web 层已在 go-live review 改用 FNV-1a，CLI 层尚未同步，是已知隐患。

## 主流程
- 裸 prompt / `run`：`run_headless` → 一次性 run → `[session <id>]`。
- `--continue`：`pick_resume_id` 取 `list_sessions(limit=1)` 最新 → resume。
- `--session <id> [--fork]`：resume 指定 id；`--fork` 先复制。
- `session show <id> [--json]`：默认按 `[role] text()` 打印（仅 Text 块）；`--json` 打印完整状态。
- `session export <id> -o <file>` / `session import <file>`：见 [agents/store](../store/index.md) 的 bundle。

## e2e 测试套件
- 入口：`scripts/e2e-glm.sh [binary]` 或 `python3 scripts/e2e_glm.py [binary]`。Flag：`--skip-web`（跳过 serve/HTTP 场景）、`--only {cli,web}`。
- binary 解析：CLI 参数 → `OPENCODER_BIN` 环境变量 → `/data/caches/opencoder-target/release/opencoder`。
- 鉴权：`ZHIPU_API_KEY` 环境变量，或 `~/.local/share/opencoder/auth.json`。
- 观测面：`opencoder session show <id> --json`（`build_session_json`）返回 `{meta, messages, subagent_tasks}`——messages 含全部 ContentBlock（Text/Reasoning/ToolUse/ToolResult），e2e 据此做深度断言而不耦合存储内部。headless 事件 marker（`▸`/`[context compacted]`/`subagent [`/`[session <id>]`）是日志断言来源。
- 断言模型：HARD = 确定性 store/契约断言（fork 拷贝完整性、bundle 往返、resume 上下文加载、plan 只读、session list/delete、config show JSON）；SOFT = 模型配合相关（工具调用 marker、压缩摘要内容、subagent 派发、reasoning_content 持久化），模型不配合时记 skip 而非 fail。
- 语法 gate（无 API key 也可跑）：`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py`——仅校验 Python 语法，不执行场景。
- 不属于 `cargo test --workspace`——需真实 API key + glm5.2 模型调用，手动 / CI 触发。
- 场景清单：E1 写文件+py_compile / E2 --continue 恢复上下文 / E3 压缩触发 / E3b 压缩后续跑 / E4 subagent 派发+DB 追踪 / E5 --fork 拷贝+不污染原 / E6 跨游戏回归 / E7/E9 models 显示 / E8 bundle 导出导入往返 / E10 plan agent 只读 / E11 web steer+queue 两段式 delivery / E12 session list+delete 生命周期 / E13 交错思考 reasoning_content 持久化 / E14 config show 合法 JSON / E15 cancel（--interrupt 中止远端 drain）/ E16 title 异步生成 / E17 崩溃恢复（fork 续跑）。
- TUI 专属功能（plan→act handoff、TaskPicker clear-all、鼠标选择、弹窗交互等）不在 e2e 覆盖范围——e2e 套件仅 CLI/HTTP 可达，TUI 交互由 `crates/tui/` 单元 + 集成测试覆盖。

## 依赖与接口
- 依赖：clap、opencoder-core、opencoder-llm（ChatClient）、opencoder-session（run/resume/generate_title）、opencoder-store、opencoder-web（serve）。
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
- e2e 场景契约：`scripts/e2e/cli_scenarios.py`（E1–E17）、`scripts/e2e/web_scenarios.py`（E11）
