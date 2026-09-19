Commit: c869027bad052c7ce91bdd0818f7be149b2b66dc

# chat 页 Agent 模式首条需求随会话创建一次提交（修复首次提交偶发 404）

## 背景

opencoder server 的 chat 页（nav menu「Agent」）在 Agent 模式下首次发送需求会偶发失败
（用户感知即「Agent 无法提交需求」）。旧链路是三步：

1. `POST /api/sessions`（`kind:'agent'`）——control 面 facade 转 `executions::submit` 派发到节点；
2. `GET /api/sessions/:id/seq` 取 SSE 游标；
3. `POST /api/sessions/:id/prompt` 经 relay 转发到节点。

节点收到 agent 执行后是**异步**初始化本地 session 行的；第 3 步的 relay 到达时本地
session 可能尚未建立，节点侧 http 处理按未知会话返回 404，首次提交即失败。

## 变更

- `crates/web/spa/src/chat.jsx` `sendSession`：Agent 模式新建会话时首条 `prompt`
  并入 `POST /api/sessions` 请求体（后端 worker `workloads/agent.rs` 早已支持
  `input["prompt"]` 在建本地 session 后原子注入并以 `initial-<id>` 幂等键提交，
  契约由 operator_e2e O5 锁定）；随后 SSE 游标直接取 0 回放全新会话，省去对节点的
  就绪往返。Operator 模式与已选会话（`dialogSel`）维持旧 create→seq→prompt 链路不变。
- `crates/web/spa/src/chat/chatMode.dom.test.jsx`：创建链路断言锁定新 wire 形状
  （Agent 模式 body 含 `prompt`，Operator 模式不带）。
- `crates/web/spa/dist/static/app.js`：产物重建。
- `agents/web/index.md`：chat 页创建链路描述回填。

## 测试

- `npx vitest run`（spa 全量）：114 文件 / 843 用例全过。
- `cargo test -p opencoder --test operator_e2e -- --test-threads=1`：O1–O5 + relay 6 过
  （O5 即「一次调用创建会话并运行首条 prompt」的端到端锁定）。
- `cargo test -p opencoder-web --lib html`：dist 产物白名单契约 7 过。
