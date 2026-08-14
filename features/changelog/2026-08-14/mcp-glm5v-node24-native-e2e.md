# Node 升级 v24.19.0：glm-5v MCP 工具真机调用原生跑通（无 polyfill）

## 背景
上一轮（status-bar-ctx-accent-and-mcp-cleanup.md）验证 `glm-vision-mcp-server` 时，本机 node v16.8.0 缺全局 `fetch`，真实 `tools/call` 只能靠 `/tmp` 一次性 undici shim 预载跑通，结论是「TUI 内使用需 Node ≥ 18」。本轮将 node 升级到 **v24.19.0（npm 11.17.0，经 n lts）**，从根源消除该约束，并完成 stdio E2E 真机验证。

## 变更
- **环境（非仓库文件）**：node v16.8.0 → v24.19.0（npm 11.17.0），全局 `npx` 可用。
- **仓库代码**：零改动。`StdioTransport` spawn 语义（newline JSON-RPC over stdio）与 MCP 配置（`.opencoder/config.json` 的 `glm-5v` 条目，gitignored）均不变，无需 env 注入 polyfill。
- **E2E 脚本（throwaway，/tmp/mcp_e2e.py，不入库）**：undici shim 由硬编码改为按 `node --version` < 18 条件应用；本轮实测 v24 走「无 shim」分支。

## stdio E2E 真机验证（GLM_MODEL=glm-5v-turbo）
与 `crates/session/src/mcp/` 同款 wire 语义（`initialize`（protocolVersion `2024-11-05`）→ `notifications/initialized` → `tools/list` → `tools/call`）：

| 步骤 | 结果 |
|------|------|
| spawn `npx -y glm-vision-mcp-server`（node v24，无 shim） | OK |
| `initialize` | OK，2.2s；server `glm-vision-mcp-server` v1.0.0 |
| `tools/list` | OK，1 个工具 `glm_5v_understand`（必填 `image`/`prompt`） |
| `tools/call`（本地 8×8 红色 PNG 路径 + 描述问句） | OK，`isError=false`，返回 "solid red image…"，30 in / 125 out（model: glm-5v-turbo） |

- **无 `fetch is not defined`**：node v24 原生全局 `fetch`，上轮 ⚠️ 环境限制解除。
- 上轮记录的「无需回退 glm-4.5v、无 1214」结论在 v24 下复验仍成立。

## 测试覆盖（rules/01/02）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| MCP 工具对 act agent 可见 / 对 subagent 隐藏（本轮未改，回归守护） | `mcp_tools_visible_to_act_agent` / `mcp_tools_hidden_from_subagent` | session/src/runner/llm_call.rs |
| MCP client 握手 / list / call wire 语义（本轮未改，回归守护） | `initialize_handshake` 等 ×26（`mcp` 过滤） | session/src/mcp/* |
| TUI /mcp 菜单 + patch + outcome（本轮未改，回归守护） | `dispatch_mcp`、`left_arrow_toggles_selected_and_stays_open` 等 ×24（`mcp` 过滤） | tui/src/command.rs、tui/src/mcp_menu/*、tui/src/app_loop_tests/mcp_outcome_tests.rs |

- 定向回归：`cargo test -p opencoder-session mcp` → 26 passed / 0 failed；`cargo test -p opencoder-tui mcp` → 24 passed / 0 failed。
- 全量回归：`cargo test --workspace` → **2544 passed / 0 failed**（156 suites 全 ok；较上轮基线 2436 净增 108，为 question-multiple-per-turn 轮新增，本轮代码零改动）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 构建门：本轮零代码改动，沿用工作区已验证构建产物（全量测试即经完整编译）。

## 风险与后续
- E2E 脚本为一次性验证器（密钥仅运行时读 env、输出已脱敏），不落入仓库；如需常驻可后续提炼为 `scripts/e2e` 用例。
- node v24 由 nvm/n 管理，若 CI 或他机回退 < 18，`glm_5v_understand` 实调用仍会缺 `fetch`（配置层无需变，属环境前提）。
