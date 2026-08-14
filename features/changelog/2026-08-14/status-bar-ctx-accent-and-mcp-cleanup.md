# 状态栏 thr/ctx 标签改 accent 色 + MCP 配置精简

## 背景
- 状态栏 `thr` 标签与 `ctx (used/limit)` 计数原用 `theme::light_blue()`；按用户要求改为「跟随中…」标签同款 `theme::accent()`（dark 主题下 Cyan），仪表条 + 百分比保持阈值红黄绿语义色不变。
- 全机 MCP 精简：实测 `npx -y glm-vision-mcp-server` 真实可用；`npx -y @z_ai/mcp-server@latest` 在本机（npm 7.21 / node 16）bin 链接缺失（`zai-mcp-server: not found`），不可用，全部清理。

## 变更

### TUI（仓库代码）
- `crates/tui/src/render_status.rs`：`"thr "` 标签 span 与 `ctx (used/limit)` 计数 span 由 `theme::light_blue()` → `theme::accent()`（与 `render.rs` 底部「跟随中…」跟随指示器同色）；注释同步改写。仪表条 + 百分比仍走 `theme::context_meter()` 阈值语义色。
- `crates/tui/src/render_tests/status_ctx.rs`：`status_bar_colors_split_between_meter_and_labels` 中 `thr`/`ctx` 标签断言从 `light_blue()` 改为 `accent()`，断言文案与 doc 注释同步；meter/percent 的 `err_color` 断言不动。

### MCP 配置（gitignored 本地配置，非仓库文件）
- 项目内 `.opencoder/config.json`：删除损坏的 `zai-vision` 条目；仅保留 `glm-5v` 并置 `enabled: true`（`npx -y glm-vision-mcp-server`，`ZAI_API_KEY: {ZHIPU_API_KEY}`，`GLM_MODEL: glm-5v-turbo`）。
- 全局 `~/.opencoder/config.json`：删除 `zai-vision`（避免项目外运行时 spawn 报错）。

## MCP 真机验证附注
- 以 `StdioTransport` 同款语义 spawn（newline-delimited JSON-RPC over stdio，`initialize`（protocolVersion `2024-11-05`）→ `notifications/initialized` → `tools/list` → `tools/call`）：握手 + 工具列表全通，往返 ~1–2.4s。
- `glm-vision-mcp-server` v1.0.0，唯一工具 `glm_5v_understand`：`image`（本地文件路径或 URL，非裸 base64）+ `prompt` 必填；`detail/max_tokens/temperature/thinking` 可选。
- `GLM_MODEL=glm-5v-turbo` 实测可用：8×8 红色 PNG + 描述问句 → 正确描述返回，30 in / 94 out（model: glm-5v-turbo），**无 1214，无需回退 `glm-4.5v`**。
- ⚠️ 环境限制（**已解除**，见同日 `mcp-glm5v-node24-native-e2e.md`）：该包声明 `engines: node >=18` 并使用全局 `fetch`；当时本机 node v16.8.0 下真实模型调用报 `fetch is not defined`（经 /tmp 一次性 undici shim 预载跑通）。后 node 升级 v24.19.0，无 shim 原生跑通。
- `@z_ai/mcp-server@latest` 弃用依据：本机 npm 7.21 下安装后 bin 链接缺失，spawn 即 `zai-mcp-server: not found`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| thr/ctx 标签 accent 色（meter/percent 保持阈值色） | `status_bar_colors_split_between_meter_and_labels` | tui/src/render_tests/status_ctx.rs |
| ctx 百分比/分母语义回归（本轮未改，回归守护） | `status_bar_shows_ctx_percent` | tui/src/render_tests/status_ctx.rs |
| 状态 chip 宽字符宽度（本轮未改，回归守护） | `status_chip_width_accounts_for_wide_emoji` | tui/src/render_tests/status_ctx.rs |

- 全量回归：`cargo test --workspace` → **2436 passed / 0 failed**（151 suites，8 crate 归属核对）；本轮测试净增 0（仅既有断言改色）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- `cargo build --workspace` → 零错误。

## 风险与后续
- ~~本机 Node 16 为 `glm_5v_understand` 实调用的硬约束~~ 已解决：node 升级 v24.19.0 后原生可用（见 `mcp-glm5v-node24-native-e2e.md`）。
- 仪表条若也要统一 accent（放弃阈值红黄绿），只需把 `render_status.rs` 中 `ctx_color` 一并替换——本轮按指令保持语义色。
