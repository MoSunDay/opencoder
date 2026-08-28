Commit: (working-tree, 内网 HTTP 原生可用——纯 JS 签名回退 + 明文警示移除)

# 内网 HTTP 原生可用：纯 JS 签名回退 + 明文 HTTP 警示移除

## 背景

LAN 部署验收（`http://192.168.31.159:18733`）暴露两问题：

1. **signature mismatch**：非 localhost 的明文 `http://` 源是 insecure context，`crypto.subtle` 为
   undefined（chromium 实证 `isSecureContext:false`），`sign.js` 的 WebCrypto 签名路径必炸——
   内网 HTTP 直连完全不可用；
2. 明文 HTTP 警示 banner 对内网部署是纯噪音，产品决定移除（内网 HTTP 是一等公民，非常态告警对象）。

## 实现

- `crates/web/spa/src/sha256.js`（新增，88 行）：纯函数 SHA-256 + HMAC-SHA256（无类、无副作用，
  K/H 常量与 Rust `auth_sig.rs` 同源锚定），供 insecure context 回退。
- `crates/web/spa/src/sign.js`：探测 `crypto.subtle` 缺失即走纯 JS 回退，`sha256Hex`/`hmacSha256Hex`/
  `signRequest` 对外接口与输出不变（小写 hex）。
- `crates/web/spa/src/login.jsx` + `main.jsx`：删除 `InsecureHttpAlert`/`isInsecureOrigin`/
  `HTTP_WARNING` 与 banner 挂载点。
- `crates/web/spa/dist/static/app.js`：重建（esbuild 将 `0x6a09e667` 收敛为十进制 `1779033703`，
  `subtle` 属性访问被提升为短变量——grep 老标识符会假阴性，对拍以行为为准）；
  `scripts/check-spa-drift.sh` → `spa dist: no drift`；served 与 dist 逐字节一致（`cmp`）。
- `scripts/browser-acceptance.js`：CDP 断链阻断规则由硬编码 `*127.0.0.1:PORT/*` 改为
  `` `*${new URL(BASE).host}/*` `` 派生——换监听主机不再必失败。
- `agents/web/index.md`：签名协议条目补 insecure-context 回退语义（repair-on-touch）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| canonical 四行串构造 | `is METHOD \\n path \\n ts \\n body-hash` | `spa/src/sign.test.js` |
| canonical 方法大写化/空体哈希/query 入签 | `uppercases the method` 等 | `spa/src/sign.test.js` |
| SHA-256 标准向量（锚定 Rust 常量） | `matches the empty-input vector` / `matches the "abc" vector` | `spa/src/sign.test.js` |
| HMAC RFC 4231 向量 | `matches the RFC 4231 HMAC-SHA256 vector` | `spa/src/sign.test.js` |
| 纯 JS 回退 SHA-256/HMAC（含 >64B key、多块输入对拍 WebCrypto） | `sha256.js pure-JS fallback` describe 组 5 例 | `spa/src/sign.test.js` |

- spa vitest：`npx vitest run` → **27/27**（sign 13 + reduce 14）。
- 跨实现对拍：openssl `dgst -sha256 -hmac` 与 JS 回退签名逐字节一致；修正探针后
  新签 `GET /api/health` → 200 `{"ok":true,"commit":"a35c96a"}`。
- 浏览器全链路（对 00:10 release 构建、`0.0.0.0:18733` 实跑）：
  `node scripts/browser-acceptance.js` → **8/8 PASS**（登录→流式→断链离线徽标→重连→中断→compact→回放）。
- 负例矩阵复测：缺头/±1h/错签 401、同签重放 409。
- 行数 gate：新增 88 行；改动 71/84/85/81/297 行，全部 <400。

## 边界

- **cargo 全量回归本轮未跑绿，原因与本轮无关**：另一并行重构（`plan`→`sandbox` agent 改名 +
  `plan_snapshot` 持久化拆除）于本轮收尾后（00:18–00:19）落入 working tree，尚未传播到下游——
  `opencoder-session` 现存 28 个编译错误（`AgentKind::Plan`/`plan_snapshot` 等已删符号）。
  本轮未触碰其任何文件（core/store 两层自身 `cargo build -p` 通过）；lint/build/test gate 待该
  重构收敛后由其归属迭代执行。浏览器验证基于改名前 00:10 的 release 二进制（含本轮全部 SPA 变更）。
- WebCrypto 路径在 secure context 下仍是首选（原生实现更快）；回退仅在 `subtle` 缺失时启用。
- smoke_nodes / 真实 LLM 会话验证沿用本轮同版二进制证据（checkpoint 全✅ / 47 帧落库）。
