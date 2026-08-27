Commit: (working-tree, 验收收尾——驱动入库 + favicon 静默 + 记忆修复)

# 浏览器验收收尾：驱动脚本入库 + favicon 404 静默 + 本地记忆修复

## 背景

上一轮（[daemon-unify](../2026-08-27/daemon-unify.md)）浏览器验收抓出 5 缺陷并全量修复推送（`dfbdc22`），遗留三项收尾：

1. 验收驱动脚本躺在 `/tmp/uitest/drive.js`，环境特定（硬编码 token 路径/截图目录），无法复跑；
2. 浏览器对 `/favicon.ico` 的自动请求被签名豁免后落入 404，服务器日志每轮刷噪音（豁免名单里有它却没有伺服路由）；
3. 中断终帧契约等新语义未沉淀进 `agents/*` 本地记忆，下轮检索仍会拿到「cancel 两处生效 / Done=正常完成」的旧模型。

## 实现

- `scripts/browser-acceptance.js`（297 行）：`/tmp/uitest/drive.js` 参数化入库。8 步确定性时间线不变（登录 → 长跑 → `ss -K` 真实断链 → 离线徽标 → 在线中断 → 断链重连续跑 → compact API → 转录回放），环境全部可覆盖：`BASE` / `OC_TOKEN`（或 `TOKEN_FILE`）/ `SHOTS` / `CHROME_PATH`，头部注释写明依赖（playwright-core + 系统 chromium `--no-sandbox`）与断链原理（CDP offline 模拟断不掉已建立的 loopback 连接，必须内核 RST）。退出码 0 当且仅当 8/8 PASS。每个签名 API 调用的响应码入环形缓冲，步骤失败时随 FAIL 行/`api-calls` 摘要输出（首轮验证曾遇 06 中断未生效的环境性竞态——服务器侧孤儿会话双跑；加诊断后干净 daemon 复跑 8/8，事后可归因而非盲重）。
- `crates/web/spa/index.html`：加 `<link rel="icon" href="data:," />`——浏览器不再发起 `/favicon.ico` 请求，404 噪音从源头消失；`data:` URI 不触发 `html.rs` 的外部引用守护（只查 `http`/`//cdn` 形态）。dist 重建 + `check-spa-drift.sh` 无漂移（仅 HTML shell +1 行，app.js/css 无变化）。
- 本地记忆修复（repair-on-touch，非新增覆盖）：
  - `agents/session/index.md`：`cancel` token 生效点由「两处」改「四处」（循环头 / mid-tool / LLM 流式中 / steer Cancelled 路径），并沉淀中断契约——`Done` 是关 SSE 流的终帧而非「正常完成」标记，任一出口缺帧订阅方永远等不到收束；
  - `agents/web/index.md`：补 SSE 重连契约（`?after=lastSeq` 有界尾部重放、`REPLAY_CAP_FRAMES=400` 封顶、终帧恒为 head）、`GET /api/sessions/:id` 的 `draining: bool`、wire `ContentBlock` serde `tag="kind"` 与 SPA 双 tag 兼容。

## 测试覆盖

- 既有测试零回归：全量 `cargo test --workspace` 通过；spa vitest 22/22（sign 8 + reduce 14）；`cargo clippy --workspace --all-targets -- -D warnings` 0 警告。
- 驱动脚本 `node --check` 语法通过；对提交产物实跑 8/8 PASS（证据截图 `SHOTS` 目录）。
- favicon 行为验证：伺服壳 grep 到 `<link rel="icon" href="data:,"`；驱动实跑全程服务器日志零 `/favicon.ico` 命中（浏览器不再发起请求，噪音从源头消失；直接 curl 该路径仍 404——无路由与豁免行为不变）。
- `check-spa-drift.sh`：no drift。

## 边界

- 驱动脚本属手动验收工具（依赖系统 chromium + 真实 LLM 凭证），不进 CI 矩阵；CI 兜底仍是 web 合约测试 + vitest。
- `/favicon.ico` 仍豁免签名（token 输入前浏览器可能自动请求），伺服 200 与豁免行为一致。
