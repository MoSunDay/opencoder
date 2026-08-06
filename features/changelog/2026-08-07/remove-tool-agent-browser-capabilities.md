Commit: (working-tree, pre-initial-commit)

# 移除 tool agent / 浏览器 / 能力开关 / config.json 能力配置

## 背景
`tools` 伞形子代理（"tool agent"）将浏览器（`web_fetch`/`web_search`/`chrome_headless`）
与 `computer_use` 能力打包为一个可选子代理，由 `CapabilitiesConfig{browser, computer_use,
tools_subagent}` 三个开关门控，并在 `/config` 表单与 `~/.opencoder/config.json` 中持久化。

该能力簇引入了重量级依赖（obscura / V8）、复杂的特性门控与 latent 技能链路，与核心
「bash + 子代理（explore/build）」范式耦合度低。本次将其整体移除，回归精简内核：
仅保留 `act`/`plan`/`explore`/`build`/`command` 五个内建代理与 `task` 工具的 explore/build
两种子代理类型。

## 变更

### 1. core — 类型与代理定义
- **`crates/core/src/config.rs`**：删除 `CapabilitiesConfig` 结构体（含 `tool_enabled` /
  `tools_subagent_enabled`）及 `Config.capabilities` 字段；`NetworkConfig` 注释由
  "LLM + browser traffic" 改为 "LLM traffic"。
- **`crates/core/src/config/merge.rs`**：移除 `capabilities.{browser,computer_use,
  tools_subagent}` 合并逻辑与 `has_editable_key` 中的对应分支。
- **`crates/core/src/agent.rs`**：`builtin_agents()` 删除 `tools` 子代理；删除
  `base_prompt_tools`、`TOOLS_SUBAGENT_AD`、`strip_tools_subagent_ad`；`tool_preamble` /
  `BASE_PROMPT` 移除 'tools' 子代理广告行（保留 explore/build 委派行）。
- **`crates/core/src/lib.rs`**：移除 `CapabilitiesConfig` 等重导出。
- **`crates/core/src/skill.rs`**：`DEP_GATED_SKILLS` 移除 `chrome-headless`（保留 `ssh-pty`）。
- **`crates/core/src/tool_deps.rs`**：移除 chrome 检测（`CHROME_CANDIDATES` / `find_chrome` /
  `ToolDepStatus.chrome`）；`all_installed` 改为 `tmux && sentinel`。
- 删除 `crates/core/assets/skills/chrome-headless/`。

### 2. session — 工具链与运行时
- 删除 13 个浏览器/computer_use 工具及支持模块：`chrome_headless.rs`、`web_fetch.rs`、
  `web_search.rs`、`web_read.rs`、`web_extract.rs`(+tests)、`serp.rs`/`serp_engines.rs`/
  `serp_tests.rs`、`research.rs`(+tests)、`computer_use.rs`、`truncate.rs`（全部仅服务于
  浏览器工具链，移除后成为孤儿）。
- **`crates/session/Cargo.toml`**：移除 `browser` feature 与 `obscura` git 依赖。
- **`tools/mod.rs`**：`registry()` 精简为 bash/read/view_image/edit/search/ls/task/ssh_pty；
  `schema_for(tools, kind)` 去掉 caps 参数；移除能力门控测试。
- **`tools/task.rs`**：`description_for(plan)` / `parameters_for(plan)` 单参化，不再广告
  'tools' 子代理类型。
- **`prompt.rs`**：`build_system(agent, workdir, skill)` 去掉 caps 参数。
- **`runner/llm_call.rs`**：`allowed` 过滤移除 `capabilities.tool_enabled`；调用去 caps。
- **`runner/subagent.rs`**：`valid_subagent_options(plan)` 单参化；移除 'tools' 子代理拒绝
  分支；plan 模式仅允许 'explore'。
- **`tools/latent.rs`**：`LATENT_TOOLS = ["ssh_pty"]`；移除 chrome-headless 解锁逻辑。
- **`resume.rs`**：移除 chrome-headless 技能推断。

### 3. tui — `/config` 表单
- **`model_menu/config_form.rs`**：移除 `Browser`/`ComputerUse`/`ToolsSubagent` 三个开关
  字段（ORDER 14→11）；弹窗高度 18→15。
- **`model_menu/patch.rs`**：`ConfigPatch` 移除 capabilities 字段与 `to_json` 中的
  `"capabilities"` 块。
- **`model_menu/view.rs`** / **`tests/config_tests.rs`** / **`tests/cursor_editing_tests.rs`**：
  同步移除渲染与光标位置断言（行索引随字段下移重算）。
- **`app_helpers.rs`**：`build_system` 调用去 caps 参数。
- **`install_tools.rs`** / **`command.rs`**：安装/帮助文案由 "tmux + chromium" 改为 "tmux"。

### 4. cli / 文档 / 用户运行时
- **`cli/src/exit_tips.rs`**：移除 chrome-headless 提示行。
- **README.md / README.en.md**：移除 obscura/agent-browser/cua 致谢块。
- **agents/{core,session,tui}/index.md**：repair-on-touch，移除浏览器/能力/chrome/computer_use
  语义。
- **scripts/install-skills-dep.sh**：移除 chromium 安装段，仅保留 tmux。
- **~/.opencoder/config.json**：移除 `capabilities` 块。
- **~/.opencoder/skills/chrome-headless/**：删除（不再由 DEP_GATED_SKILLS 播种）。

## API 契约变更（破坏性）
- `Config.capabilities` 字段删除；`CapabilitiesConfig` 类型删除。
- `build_system` / `schema_for` / `task::description_for` / `task::parameters_for` /
  `valid_subagent_options` 均移除 caps/tools_on 参数。
- `builtin_agents()` 不再返回 `tools` 子代理；`task` 工具不再接受 `subagent_type:"tools"`。

## 测试覆盖（当次实跑）
- `cargo build --workspace` → Finished，0 error
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 warning
- `cargo test --workspace` → 1895 passed / 0 failed
  - opencoder-core: 151 / opencoder-session: 469 / opencoder-tui: 952
  - opencoder-cli: 82 / opencoder-web: 65（余为 store/llm/client/doctest）
- 删除的测试均为被移除特性专用（capabilities_and_tools.rs、web_extract/research/serp/
  chrome_headless/computer_use 内联测试、agent.rs tools-subagent 测试等）。
- 新增/保留测试覆盖：`task` 工具 explore/build schema、subagent 拒绝未知类型、
  latent ssh_pty、config 表单剩余字段 round-trip 与光标定位（行索引已重算）。
