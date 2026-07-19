# 浏览器 / 计算机使用 / 代理集成：proxy-aware HTTP、computer-use 循环、能力门控工具集

## 背景

三件事在本轮一并落地，因它们共享同一套代理通道与能力开关：

1. **代理感知网络**：LLM 客户端与 browser 工具此前各自直连。在企业 / 代理环境下需要统一从 `Config.network.proxy`（含 `OPENCODER_PROXY`/`ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` 回退）构造 reqwest 客户端。关键是**环回旁路**——若配置了正向代理却不豁免 `127.0.0.1/localhost/::1/0.0.0.0`，本地 mock server 与自连测试在代理生效时会被截断。
2. **原生 computer-use 循环**：从 cua（`trycua/cua`）的 perceive→act 周期提炼一个 backend 无关的循环，仅持有「步数预算 + 完成守卫」，故可单测；真实 provider 沙箱后端（Anthropic / OpenAI computer-use）留作下一里程碑。
3. **Web 研究工具集 + 能力门控**：从 agent-browser（`vercel-labs/agent-browser`）移植 markdown 协商 + `llms.txt` 爬取 + 可读正文抽取 + DDG 解析算法；obscura 作为 headless 渲染后端支撑 `web_fetch`/`web_search`。这些可选工具经 `CapabilitiesConfig` 门控，关能力时模型在请求 schema 里根本看不到该工具。

目标：代理与能力成为 `core` 的共享横切关注点，session 只消费；可选工具按能力按需暴露，不污染默认 schema。

## 变更

### `crates/core/src/net.rs`（新增）

- `effective_proxy(explicit)`：优先级 = 显式 config 值 → `OPENCODER_PROXY` → `ALL_PROXY` → `HTTPS_PROXY`/`HTTP_PROXY`，空白值忽略。
- `build_http_client` / `build_http_client_with_read_timeout`：rustls 客户端，30s connect、可配 per-read idle 超时；有代理时挂 `Proxy::all(...)` 并以 `LOOPBACK_NO_PROXY = "127.0.0.1,localhost,::1,0.0.0.0"` 构造 `NoProxy` 挂到代理上，确保本地流量直连。
- 被 llm client 与 browser 工具共用；经 `crates/core/src/lib.rs` re-export（`build_http_client` / `effective_proxy`）。

### `crates/core/src/computer_use.rs`（新增）

- `ComputerAction`（serde 1:1 映射 provider 的 computer-use tool call，`type` + flatten 剩余字段）、`Observation`（screenshot_b64 + text + `done`）、`LoopOutcome::{Done,MaxStepsReached}`。
- `ComputerUseExecutor` trait（`initial_observation` / `execute`）：provider 沙箱实现真后端，测试实现 `RecordingExecutor`（记录动作、收到 `done` 即标完成）。
- `ComputerUseLoop::run`：种子观测 → 闭包给下一步动作（`None` 即停）→ execute → `done` 即停 → 否则耗尽步数预算返回 `MaxStepsReached`。
- `LlmProviderExecutor`/`ProviderBackend::{Anthropic,OpenAi}` 声明在此以完整能力面，真实执行待 sandbox 接入。

### `crates/core/src/config.rs`

- `Config` 新增 `network: NetworkConfig`、`capabilities: CapabilitiesConfig`（紧跟既有字段列表之后）。
- `NetworkConfig { proxy: Option<String> }`。
- `CapabilitiesConfig { browser: bool, computer_use: bool }` + `tool_enabled(name)`：按工具名映射到对应能力开关（关能力 → 该工具被 runner 过滤）。
- env 覆盖与 merge 分支补齐这两个字段（默认 `browser=false`，故默认不暴露 browser 工具）。

### `crates/session/src/tools/web_read.rs`（新增，always-compiled）

- 纯算法、与 feature 无关：`normalize_url`（补 https / 拒非 http(s) / 要求 host / 去 fragment）、`markdown_fallback_url`（`.md` 兄弟回退，根路径用 `/index.md`，已 `.md` 返回 None）、`llms_txt_candidates`（向根爬 `/a/b/llms.txt → /a/llms.txt → /llms.txt`）、`extract_readable_text`（html2text 剥 `<script>`/`<style>`、优先 `<main>`、合并空行）、`parse_ddg_results`（DDG HTML 结果 + `uddg=` 重定向解码 + 上限）。`BODY_LIMIT=2MB`、`READ_ACCEPT` 表达 markdown 偏好。
- 算法移植自 agent-browser 的 `cli/src/read.rs`，HTML→text 交给 `html2text`、URL 选择交给 `scraper` 以利维护。

### `crates/session/src/tools/web_fetch.rs` + `web_search.rs`（新增，feature-gated `browser`）

- 经 obscura headless 引擎渲染/抓取，再把渲染 HTML 喂给 `web_read::extract_readable_text`。
- obscura 的 V8/Rc future 是 `!Send`：在 `spawn_blocking` + `LocalSet` 内驱动，使外层 `Tool::execute` future 保持 `Send`。

### `crates/session/src/tools/computer_use.rs`（新增，always-compiled）

- 模型面向的 computer-use 工具：解析 provider 的 computer-use tool call → 构造 `ComputerAction` → 驱动 `core::ComputerUseLoop`（v1 用 `RecordingExecutor`，真后端待接）。

### `crates/session/src/runner.rs`

- **capability 过滤器**：构建请求 schema 前以 `CapabilitiesConfig::tool_enabled(name)` 过滤工具列表——能力关时该工具从请求 schema 消失（集成测试断言 schema 中不存在）。
- `ToolContext.proxy` 由 `session.config.network.proxy.clone()` 灌入（工具执行时可见代理配置）。

### `crates/core/src/agent.rs`

- 注册 `tools` umbrella 子 agent（`AgentMode::Subagent`，plan-visible，区别于 plan-hidden 的 `build`）：收拢 `web_fetch`/`web_search`/`computer_use` + 只读文件工具（read/glob/grep/ls）。act agent 经 `task` 工具调度它。配套 `base_prompt_tools()`。

### 代理贯通（生产站点，机械灌入 `network.proxy`）

- `crates/llm/src/client.rs`、`crates/web/src/api.rs`、`crates/tui/src/{app,worker}.rs`、`crates/cli/src/run.rs`、`crates/client/src/remote.rs`：构造 LLM/HTTP 客户端处改用 `build_http_client(proxy)`（或带 read 超时变体），`proxy` 取自 `Config.network.proxy`。

### `crates/tui/src/model_menu/`

- `/config` 菜单新增 `browser` + `computer_use` 能力复选框字段（`Field::Browser` 等），`ModelMenu::new` 从 Config 初始化、`build_patch` 往返；Space/Left/Right 切换且焦点不漂移。

### 文档与杂项

- README.md 新增「致谢」段：obscura（headless 依赖）、agent-browser（算法参考）、cua（循环参考）。
- `.gitignore` 追加 `reference/`（克隆的外部参考仓）。

## 测试覆盖

> 全部为新文件 / 新模块的新增测试，按 test-pyramid 的单测 / 集成层组织，**确定且无网络**。

### `crates/core/src/net.rs`（6 单测）

| 测试 | 验证不变式 |
|------|-----------|
| `explicit_proxy_wins_over_env` | 显式 config 代理覆盖环境变量 |
| `empty_explicit_falls_through` | 空显式值回退 env；无 env 时返回 None（env 隔离后验证） |
| `socks5_url_parses_as_reqwest_proxy` | socks5/socks5h/http(s) 代理均可构造 `reqwest::Proxy`（socks feature 已接） |
| `loopback_no_proxy_is_constructable` | 环回排除列表可构造 `NoProxy` |
| `build_http_client_with_proxy_still_builds` | 代理 + loopback no_proxy 客户端可构造 |
| `build_http_client_direct_when_no_proxy` | 无代理（清 env）时直连客户端可构造 |

### `crates/core/src/computer_use.rs`（3 单测，`RecordingExecutor` 作测试替身）

| 测试 | 验证不变式 |
|------|-----------|
| `loop_stops_on_done_action` | executor 标 `done` 即停、`outcome=Done`、动作计数正确 |
| `loop_stops_when_closure_returns_none` | 决策闭包返回 `None`（无更多动作）即停 |
| `loop_respects_max_steps` | 步数预算耗尽返回 `MaxStepsReached` |

### `crates/core/src/agent.rs`（1 单测）

| 测试 | 验证不变式 |
|------|-----------|
| `tools_subagent_is_registered_with_capability_tools` | `tools` 子 agent 解析成功且为 `Subagent` 模式；携带 `web_fetch`/`web_search`/`computer_use`/`read`/`glob`/`grep`/`ls`；act 与 plan prompt 均广告 `'tools' subagent` |

### `crates/session/src/tools/web_read.rs`（14 单测）

| 测试 | 验证不变式 |
|------|-----------|
| `normalize_adds_https_and_strips_fragment` | 补 `https://`、去 fragment |
| `normalize_rejects_non_http` | 拒 `file:///`、`ftp://` 等非 http(s) |
| `normalize_requires_host` | 无 host 报错 |
| `markdown_fallback_appends_md` | `.md` 兄弟回退，query 保留 |
| `markdown_fallback_root_uses_index` | 根路径回退 `/index.md` |
| `markdown_fallback_none_when_already_md` | 已 `.md` 不再回退 |
| `llms_candidates_crawl_to_root` | `llms.txt` 候选自深向根爬 |
| `llms_candidates_root_target` | 根目标仅 `/llms.txt` |
| `extract_readable_prefers_main_over_body` | 优先 `<main>`、`<script>` 不泄漏 |
| `extract_readable_drops_script_and_style` | 剥离 `<style>`/`<script>` |
| `extract_readable_collapses_blank_lines` | 合并多余空行 |
| `parse_ddg_extracts_title_url_snippet` | DDG 结果解析 title/url/snippet（含 `uddg=` 重定向解码） |
| `parse_ddg_respects_limit` | 结果上限生效 |
| `parse_ddg_handles_empty_and_non_ddg_href` | 空页 / 非 DDG href 安全处理 |

### `crates/session/tests/capabilities_and_tools.rs`（4 集成测试，`MockChatClient` 零 token 驱动）

| 测试 | 验证不变式 |
|------|-----------|
| `capability_gate_hides_computer_use_when_disabled` | 能力关 → `computer_use` 不进请求 schema，read-only 工具仍在 |
| `capability_gate_exposes_computer_use_when_enabled` | 能力开 → `computer_use` 进请求 schema |
| `tools_subagent_is_dispatchable_from_act` | act 经 `task` 调度 `tools` 子 agent，无 "Unknown subagent_type"，子 agent 发起自己的 LLM 调用 |
| `config_save_load_round_trips_capabilities` | capabilities patch `Config::save` → `Config::load` 往返，并可覆盖关闭 |

### `crates/tui/src/model_menu/mod.rs`（能力面新增 2 单测）

| 测试 | 验证不变式 |
|------|-----------|
| `model_menu_inits_capabilities_from_config` | 菜单从 Config 读 browser/computer_use，`build_patch` 往返且默认 `false` |
| `toggling_browser_capability_checkbox` | Space/Left/Right 切换 browser 能力复选框，焦点不漂移 |

> 诚实说明：obscura 支撑的 `web_fetch`/`web_search` **无 live-network e2e 测试**。按 test-pyramid 规则，单测 / 集成层确定且无网：`web_read` 的纯算法有 14 项覆盖、代理通道有环回旁路验证；`browser` feature 编译干净，但真实抓取需联网，留待后续网络 e2e。`ComputerUseLoop` 的真 provider 后端执行同样未接真实沙箱（`RecordingExecutor` 替身覆盖循环控制语义）。

## Gate（当次实跑取证）

| 项 | 结果 |
|----|------|
| `cargo build --workspace` | exit 0，干净 |
| `cargo build -p opencoder-session --features browser` | exit 0，干净（obscura 经 `spawn_blocking`+`LocalSet` 编译通过，`!Send` future 隔离在阻塞线程） |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0，零警告 |
| `cargo test --workspace` | **664 passed / 0 failed，跨 57 个 binary** |
| 代理环回旁路 | 设 `HTTP_PROXY/HTTPS_PROXY/ALL_PROXY=http://127.0.0.1:9` 且**不设** `NO_PROXY`：`stream_timeout`（2/2）+ web 契约测试（15/15）全过——localhost 流量绕过 bogus 代理，未截断 |

> 本次新增测试合计 30 项（net 6 + computer_use 3 + web_read 14 + capabilities_and_tools 4 + model_menu 2 + core/agent 1）。`cargo test --workspace` 的 664 计数含 bash 修复迭代（见同目录 `bash-tool-detach-controlling-terminal.md`）的新增项，故非纯本次 delta。
