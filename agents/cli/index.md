Commit: (working-tree, sandbox 回退为 plan：恢复 plan/act 双模式回切，写拦截能力保留)

# cli 模块

## 职责
clap 命令前端 + headless 运行时。解析全局 flag 与子命令（run/tui/ts/daemon/config/models/session/todos/update/install-tools，`ts` 别名 `rs`），把用户意图分发到 session/web/store/todos 层。headless 模式（`run` 或裸 prompt）是 e2e 与脚本化的主入口。

## 边界与非目标
- 不做终端渲染（TUI 在 `opencoder_tui`）、不做 HTTP 服务实现（web 在 `opencoder_web`）。
- 不持有长期运行态——headless 一次性 run 完即退；`daemon` 子命令仅打印迁移指引（`opencoder-server` / `opencode-agent`），不再内嵌 web。
- 非目标：headless `run` 不直接暴露 steer/queue 两段式 delivery（那是 web `POST /prompt` 的 `delivery` 字段；`--delivery` 旗标转发随已删除的 `client` 子命令移除，远端节点模式由 `opencode-agent` 二进制承接）；CLI headless 单 prompt。

## 关键抽象
- `Cli`（`src/lib.rs`）：全局 flag `--model/--agent/--image/--workdir/--session/--continue/--fork/--verbose/--prompt-file`（`--agent` 以内建名覆盖本次运行的 agent，`parse_agent_name` 在 clap 解析期校验——interlude 已移除的 `sandbox` 名报错并提示 renamed back to `plan`；新会话作为 primary agent 持久化该选择；resume 时显式选择胜过已存 agent 并重新持久化；store 中 interlude legacy `agent='sandbox'` 行在读路径归一化为 `plan`，见 [agents/store](../store/index.md)） + `Command::{Run, Tui, Ts, Daemon, Config, Models, Session, Todos, InstallTools, Update}`（`Update` 无参，用内置提示词经 `run_headless` 委托代理执行自更新：clone latest main → build → 原子替换 PATH 二进制；`InstallTools` 见 `cli/src/install_tools.rs`，探测+安装 tmux 等可选依赖）（`Ts` 别名 `rs`；裸 `ts`/`rs` 在 tmux 内外都恒新建 `opencode-<ulid>` tmux session，tmux 内用 detached-create + `switch-client`，不再退化为 inline TUI。`registry.rs` 打开**中央索引** `<data_root>/ts.db`（`TsRegistry`，store crate），首次缺少 `migrated` 标记时做一次性迁移：分页扫描各 per-workdir store，把 `model IS NULL`（旧 ts 标记）会话导入 registry，幂等 upsert + 末尾打标记、崩溃安全；此后 `-l`/`-r`/`-d`/`-c` 全部只查索引，不再扫 store。`register()`（`ts_start`/TUI 镜像）只写 registry，不再写 per-workdir `workdir` marker；`-l` 先列 live tmux（真实 workdir 来自 pane path）再列 registry stopped 行，marker 图例 `*`=attached、`·`=live(detached)、`-`=stopped；排序：非 stopped（attached/live）优先，再按 workdir 路径升序、组内按创建时间倒序；`-r <id>` / `-d <id>` 把前缀在 live tmux 与 registry 中解析为唯一完整 ID（跨 store 重复 id 由 `INSERT OR REPLACE` 收敛，不再报歧义）；`-r` 从任意目录按记录路径重连/冷启动，`-c` 清理 stopped registry 行及其 store 内容，`-d` 对非当前 live tmux 先 kill 再删；普通 `tui`/`run` 会话不进 registry（TUI 侧由 `TsMirrorStore` 在 `ts.db` 存在时镜像 ts 会话的 title/preview/delete））+ trailing `prompt`。
- `SessionSub::{List, Show{id, json}, Delete, Export{id, out}, Import{input}}`（`src/lib.rs`）。`Show --json` 是深度观测面（见下）。
- `Daemon` 统一入口（`src/daemon.rs` + `src/server.rs`）：`--server` 起 web server（registry + fleet dispatch + 本地引擎，flags `--host/--port/--web/--token`），`--client --remote <URL>` 以执行节点常驻（flags `--name/--token`）；`daemon_mode` 纯函数校验两者恰好其一（client 模式必须 `--remote`），`resolve_client_token` 解析 `--token` → `OPENCODER_SERVER_TOKEN`（节点永不自动生成），`default_node_name` 生成默认节点名——全部纯函数 + 单测。解析契约测试：`tests/cli_parse_daemon.rs`。
- `TodosSub`（`src/lib.rs` / `src/todos_cmd.rs`）：`validate --file` 做无副作用合同校验；`run --file [--debug]` 创建并执行工作流；`resume/show/events/list/interrupt` 管理 Store 中的持久化运行。`--debug` 只作用于 run/resume 的文件投影，Store 始终是权威状态。run/resume 的 stdout 是纯最终状态 JSON（`--json` 紧凑单行；**BREAKING**：`workflow_id=` 前缀与运行期进度 tailer 均在 stderr）；退出码 completed=0、本地 Ctrl-C 挂起=130、其他终态=1（携带状态名）；`events` 对未知 id 报错 exit 1，`list --limit` 默认 100；`show`/`events`/`list` 均带 `--json`（结构化观测面），`events --after <seq>` 经 `todo_events_after` 增量拉取。
- `ConfigSub::{Show, Set{model}}`（`src/lib.rs`）。`Config Show` 输出合并后配置 JSON（stdout 保持纯 JSON 可机器消费；激活命名环境时先向 stderr 打 `active env: <name>` 一行，`active_env_banner()`）；`Config Set <model>` 经 `config_dispatch` → `Config::save` 把 `provider/model_id` 写回 opencoder.json（设全局默认模型的脚本化入口；与 TUI `/model` 的 `y=global` / Web `POST /model persist_default=true` 语义一致）。
- `run_headless`（`src/run.rs`）：建/恢复 SessionState → `run(session, prompt, print_event)` → 异步 `generate_title`。prompt 先经 `rewrite_legacy_plan_prefix` 把前导 `/plan`/`/plan …` 重写为 `/plan …`（legacy 兼容）。`--continue` 取最新 session；`--session <id>` 指定；`--fork` 在 resume 前调 `fork_session` 复制（原 session 零修改）。**resume 摘要**：resume 后调 `print_resume_summary(&session).await`，蓝字单行 `⤷ resumed session: done/total subagents done — ✔explore … ✘build …`（空→不打印）；格式逻辑抽出为纯函数 `pub(crate) fn format_resume_summary(&[SubagentTaskRecord]) -> Option<String>` 便于单测。
- `fork_session(store, parent_id)`（session crate `crates/session/src/fork.rs`，CLI 仅调用）：读 parent meta+messages → 新 id → `create_session` + `append_messages`，返回新 id，打印 `[forked P → C]`。
- `print_event`（`src/display.rs`）：headless 事件渲染——`▸ name input`（ToolStart，input 取 command/path/description 摘要）、缩进输出（ToolEnd，错误红色）、`[context compacted] summary`、`⤷ subagent [kind] prompt` / `✔|✘ summary`、`[switched to X mode]`、`[session <id>]`（run 结束）、`[status]`。`LlmRoundStart`/`LlmRoundEnd`/`ReasoningDelta`/`SubagentChild` 不打印；`QueueConsumed`/`SteerConsumed` 打印 prompt 原文，`TranscriptReset` 打印折叠横幅 `transcript_reset_banner`（`── context cleared (N messages folded) ──`，旧 plan-handoff 横幅的替代），另有 `CompactionDelta`（增量摘要）、`ModelSwitch`、`AutoPilot`（phase/iteration）marker。这套 marker 是 e2e 日志断言的稳定来源。
- `build_session_json(store, id)`（`src/session_cmd.rs`）：返回 `{meta, messages, subagent_tasks}` 的 JSON 值——meta 含 compaction summary 字段；messages 含**全部** ContentBlock（Text/Reasoning/ToolUse/ToolResult，不过滤）；subagent_tasks 含 status/result/ok。ClearContext sentinel 在 meta 中脱敏为 None（绝不外泄，`build_session_json_redacts_clear_context_sentinel` 保护）。`session show <id> --json` 打印之。这是 e2e 深度断言的机器可读观测面，解耦存储内部（不依赖 sqlite 直查 / hash 路径）。
- `data_dir_for(workdir)`（`opencoder-core` 的 `src/data_dir.rs`，经 lib.rs re-export；CLI/web/tui 共用同一实现）：workdir → 本地数据目录（`<data_local>/opencoder/<hash>/opencoder.db`）。

## 主流程
- 裸 prompt / `run`：`run_headless` → 一次性 run → `[session <id>]`。
- `--continue`：`pick_resume_id` 取 `list_sessions(limit=1)` 最新 → resume。
- `--session <id> [--fork]`：resume 指定 id；`--fork` 先复制。
- `session show <id> [--json]`：默认按 `[role] display||text()` 打印（仅 Text 块；display 为 verbatim 原文，旧行回退 text()，`show_message_line_prefers_display_then_blocks`）；`--json` 打印完整状态。
- `session export <id> -o <file>` / `session import <file>`：见 [agents/store](../store/index.md) 的 bundle。

## e2e 测试套件
- 入口：`scripts/e2e-glm.sh [binary]` 或 `python3 scripts/e2e_glm.py [binary]`。Flag：`--skip-web`（跳过 serve/HTTP 场景）、`--only {cli,web}`。
- binary 解析：CLI 参数 → `OPENCODER_BIN` 环境变量 → `CARGO_TARGET_DIR/release/opencoder` → 仓库 `target/release/opencoder`（缺省不存在则报错退出，不再硬编码机器路径）。
- 鉴权：`ZHIPU_API_KEY` 环境变量，或 `~/.local/share/opencoder/auth.json`。
- 观测面：`opencoder session show <id> --json`（`build_session_json`）返回 `{meta, messages, subagent_tasks}`——messages 含全部 ContentBlock（Text/Reasoning/ToolUse/ToolResult），e2e 据此做深度断言而不耦合存储内部。headless 事件 marker（`▸`/`[context compacted]`/`subagent [`/`[session <id>]`）是日志断言来源。
- 断言模型：HARD = 确定性 store/契约断言（fork 拷贝完整性、bundle 往返、resume 上下文加载、plan 只读、session list/delete、config show JSON）；SOFT = 模型配合相关（工具调用 marker、压缩摘要内容、subagent 派发、reasoning_content 持久化），模型不配合时记 skip 而非 fail。
- 语法 gate（无 API key 也可跑）：`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py`——仅校验 Python 语法，不执行场景。
- 不属于 `cargo test --workspace`——需真实 API key + glm5.2 模型调用，手动 / CI 触发。
- 场景清单：E1 写文件+py_compile / E2 --continue 恢复上下文 / E3 压缩触发 / E3b 压缩后续跑 / E4 subagent 派发+DB 追踪 / E5 --fork 拷贝+不污染原 / E6 跨游戏回归 / E7/E9 models 显示 / E8 bundle 导出导入往返 / E10 只读 agent 不可写盘（plan 只读契约） / E11 web steer+queue 两段式 delivery / E12 session list+delete 生命周期 / E13 交错思考 reasoning_content 持久化 / E14 config show 合法 JSON / E15 web interrupt（`POST /interrupt` 中止运行中 drain，cancel token 按 drain 刷新 + 会话存活）/ E16 title 异步生成 / E17 崩溃恢复（kill -9 后 `--continue` 跨进程续跑）/ E18 autopilot 自驱动循环 / E19 todos run→resume→observe / E19b todos DAG 并发+--debug 投影+events --after 游标 / E19c todos 外部 interrupt→resume 自愈（挂起 rc==1） / E20 config_scenarios 免 key 配置面（env 覆盖含 providers 注册表 / api_key 掩码 / envs 激活 banner / malformed `--model` 拒绝）。
- TUI 专属功能（Shift+Tab 折叠派发、TaskPicker clear-all、鼠标选择、弹窗交互等）不在 e2e 覆盖范围——e2e 套件仅 CLI/HTTP 可达，TUI 交互由 `crates/tui/` 单元 + 集成测试覆盖。

## 依赖与接口
- 依赖：clap、opencoder-core、opencoder-llm（ChatClient）、opencoder-session（run/resume/generate_title）、opencoder-store、opencoder-web（serve）。
- 被依赖：binary crate（`src/main.rs` 解析 `Cli` 并分发）。

## 相关模块
- [agents/session](../session/index.md) — headless run/resume/fork 的核心。
- [agents/store](../store/index.md) — session 子命令 + bundle 导出导入。
- [agents/web](../web/index.md) — `opencoder-server` 二进制的 HTTP 服务实现（原 `daemon --server`）。
- [agents/todos](../todos/index.md) — todos 子命令的工作流运行时。

## 代表性锚点
- 深度观测面测试：`session_cmd::tests::build_session_json_emits_meta_messages_and_subagent_tasks` / `build_session_json_errors_on_missing_session`
- fork 实现测试：`cli/tests/fork_session.rs`
- CLI 解析测试：`cli/tests/cli_parse.rs`（含 `session show --json` 解析）
- headless 事件渲染：`run::tests::{summarize_input_extracts_command, truncate_adds_ellipsis}`
- e2e 场景契约：`scripts/e2e/cli_scenarios.py`（E1–E19）、`scripts/e2e/todos_scenarios.py`（E19b/E19c）、`scripts/e2e/web_scenarios.py`（E11/E15/E18b）、`scripts/e2e/config_scenarios.py`（E20，免 key 配置面）
ios.py`（E20，免 key 配置面）
