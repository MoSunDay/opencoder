# 清理 autopilot 移除后的残留：SPA dist 回放臂与运行时配置

一句话：上一轮移除 autopilot 后，提交树里的 `crates/web/spa/dist/static/app.js`
仍内嵌 `case "autopilot"` 历史回放臂（源码 `reduce.js` 已无该臂），运行时
`~/.opencoder/ap.json` 与 `config.json` 的 `autopilot` 键也仍残留；本次重建
SPA dist 清除内嵌臂，并删除运行时 ap 配置。

## 改动

- SPA dist：在 `crates/web/spa` 执行 `npm run build` 重建，
  `dist/static/app.js` 4,508,658 → 4,508,460 字节，
  `rg -i autopilot dist/static/app.js` → 0；源码与 `app.css` 无 diff。
- 行为变化：旧会话 SSE 中的 `autopilot` 帧在 SPA 侧不再有渲染臂（忽略），
  TUI 侧维持 warn + 跳过；后端历史 payload 仍原样透传，不删除会话数据。
- 运行时配置（仓库外）：删除 `~/.opencoder/ap.json`；从
  `~/.opencoder/config.json` 移除 `autopilot` 键（`Config::save` 会保留未知
  键，旧键不会自动消失）。`config.json.bak.*` 备份未改动。
- 仓库记忆与代码：`repo-memory.md`、`agents/**`、`features/index.md` 无 ap
  引用；`crates/store` 的 v40 迁移保留 `autopilot_mode` 列名用于旧库清理。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 内嵌 dist 壳路由与固定资产白名单 | `shell_is_the_console_entry`、`static_whitelist_serves_fixed_build_outputs` | `crates/web/src/html.rs` |
| 内嵌 dist 可编译并启动 Web 路由 | `web_enabled_serves_html_route`、`web_disabled_omits_html_route` | `crates/web/src/lib.rs` |
| dist 不含 autopilot 回放臂 | 构建产物检查 `rg -i autopilot dist/static/app.js` | `crates/web/spa/dist/static/app.js` |

- 定向回归：`cargo test -p opencoder-web --lib html` → 7 passed / 0 failed。
- 构建：`cd crates/web/spa && npm run build` → 成功（8563 modules，30.35s）。
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
