# --prompt-file：用文件自定义 agent 系统提示词（自动追加 bash+subagent 前言）

## 背景

此前 agent 的系统提示词完全由内置 prompt 决定，用户若想把 opencode 变成特定领域的编码助手（注入自定义「角色提示词」），没有入口。同时旧的 `--model` / `--small-model` / `--agent` 三个全局 flag 与 `opencoder.json` 配置职责重叠、且绕过 provider 解析逻辑，已废弃——模型与 agent 改为只从配置解析。

## 设计要点

- **`--prompt-file <PATH>`（全局 flag）**：读取该文件全文作为 agent 系统提示词主体，末尾自动拼接 [`tool_preamble()`](../../crates/core/src/agent.rs)，补齐 bash / subagent 使用说明，保证自定义提示仍能正确驱动工具调用与子代理委派。
- **前言可裁剪**：`tool_preamble()` 含精确的 `TOOLS_SUBAGENT_AD` 行（`'tools' subagent`）与 `'build'` 委派子串，与内置 `base_prompt` 保持一致；因此 `strip_tools_subagent_ad` 在 `tools_subagent` 能力关闭时仍能正确隐藏该广告。
- **废弃 flag 移除**：删除 `--model` / `--small-model` / `--agent`；`run_headless` 不再从 cli 覆盖 config，agent 名固定取 `config.agent.default`，模型由 `resolve_endpoint()` 解析。

## 变更

### 新增 `tool_preamble`（`crates/core/src/agent.rs`，纯函数）
- `pub fn tool_preamble() -> &'static str`：bash + subagent 使用前言常量。
- 单测 `tool_preamble_contains_substrings`：断言前言含 `TOOLS_SUBAGENT_AD`、`'build'`、`'tools' subagent`。
- `crates/core/src/lib.rs` 重新导出 `tool_preamble`。

### CLI（`crates/cli/src/lib.rs` / `run.rs` / `tests/cli_parse.rs`）
- `Cli::prompt_file: Option<PathBuf>`（`#[arg(long, global = true)]`）。
- `run_headless`：若提供 `--prompt-file`，读文件、`trim`、拼接 `tool_preamble()` 覆写 `session.agent.prompt`。
- 移除 `Cli::{model, small_model, agent}` 字段及对应 arg。
- 单测 `prompt_file_flag_parsed`：解析 `--prompt-file x.md`；缺省为 `None`。
- `global_flags_parsed` 更新为断言 `--workdir` / `--prompt-file`。

### 文档
- `README.md` / `README.en.md`：全局选项表移除三个废弃 flag，新增 `--prompt-file` 说明。

## 已知限制

- `--prompt-file` 仅在 headless `run` 路径生效（覆盖 `session.agent.prompt`）；TUI 启动路径不读取该 flag（`opts_from_cli` 只透传 `workdir`）。
- 文件读取失败（不存在 / 无权限）以 `--prompt-file <PATH>: <e>` 报错并中止，不静默回退。

## 验证（当次实跑，cli + core scope）

| 命令 | 结果 |
| --- | --- |
| `cargo clippy -p opencoder-cli -p opencoder-core --all-targets -- -D warnings` | `Finished`，零告警（exit 0） |
| `cargo test -p opencoder-cli -p opencoder-core` | 全绿，117 passed / 0 failed |

逐套件（当次输出尾段，直接观察）：cli lib 27、cli_parse 15（含 `prompt_file_flag_parsed`）、fork_session 3、core lib 26（含 `tool_preamble_contains_substrings`）、config_contract 24、skill_contract 9、tool_filter 13；doc-tests 0。

> **本特性权威覆盖（稳定锚点，不随并行改动漂移）**：上述 cli lib / cli_parse 套件计数含并行 `ts` 工作流的测试，会随其进度漂移（本次为 27 / 15）。可直接复跑核对的两条 prompt-file 专属用例（每次均 `1 passed; 0 failed`）才是本特性的稳定验收锚点：
> - `cargo test -p opencoder-cli --test cli_parse prompt_file_flag_parsed`
> - `cargo test -p opencoder-core --lib tool_preamble_contains_substrings`

> 注：workspace 整体（session / store / tui / web）当时正被其它并行任务活跃修改，未纳入本次 cli+core 验证范围；详见本批次 review 记录。
