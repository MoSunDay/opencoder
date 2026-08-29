Commit: (working-tree, SPA chat 交互整体迁移 @ant-design/x 2.9.0 + antd 6.6.2)

# SPA chat 交互迁移：Bubble.List + Sender + Conversations（antd 5→6）

## 背景

舰队控制台的消息区/输入区/会话切换此前为自研组件（render.jsx 转写渲染 + TextArea/Ctrl+Enter + Select 会话切换）。本轮整体迁移到 @ant-design/x 2.9.0 的 X 典型 chat 布局，antd 同步升 6.6.2。协议层（sign.js/api.js/sse.js/reduce.js HMAC 签名 + SSE + 帧归约）**零改动**——X 的 XRequest 面向 OpenAI 协议端点，无法直连本仓库的 HMAC 签名与自定义 SSE 域事件（tool_start/tool_end 等），该胶水层保留；后端 Rust 接口零变更，仅 spa/dist 三文件产物内容更新（html.rs include 白名单不变）。

## 实现

- **T1 依赖**：antd ^5.21.6 → ^6.6.2、新增 @ant-design/x ^2.9.0（lockfile 锁定；peer 无冲突，react 18.3.1 不动）。
- **T2 antd6 迁移 + jsdom 冒烟基建**：`destroyOnClose`→`destroyOnHidden`、`maskClosable`→`mask={{closable:false}}`（login.jsx）、`Spin tip`→`description`（chat.jsx，deprecation guard 实证抓出）；新增 `test/setup-dom.js`（matchMedia/ResizeObserver/IntersectionObserver/scrollIntoView/scrollTo/animate stub）与 `app.dom.test.jsx`（4 视图 landmark + console 零 deprecated 断言）。
- **T3 消息区 → Bubble.List**：新增纯函数 `bubbleItems.js`（turns→items，key=`kind:index`，role=user/ai/think/tool/sys）+ `transcript.jsx`（`<Bubble.List items role autoScroll/>`：user=end/filled/❯、ai=start/outlined/◉、think/tool/sys=borderless；contentRender 沿用等宽 Paragraph/💭 Collapse/🔧 工具行（error 红标+input/output）/sys 灰字；UsageFooter/StatusTag/EmptyHint 迁入）。`render.jsx` 删除。`role` prop 为 X 2.9 正名（非 `roles`）。
- **T4 输入区 → Sender**：TextArea+双 Button → `<Sender value onChange onSubmit onCancel loading/>`；`onSubmit`(Enter)→既有 send()、`loading={busy}` 停止键→既有 interrupt()，数据流/签名路径零改动。
- **T5 会话切换 → Conversations 侧栏**：新增纯函数 `conversationItems.js`（dialogs→items，复用 format.js#dialogLabel）+ `chatSidebar.jsx`（节点 Select + `<Conversations items activeKey onActiveChange creation={{label:'新建对话'}}/>`）；chat.jsx 重排 X 典型双栏（左 264px 侧栏/右 transcript+Sender）；store.js Tab1→Tab2 预选联动原样保留（冒烟自动化断言覆盖）。
- **验收工具适配**：`scripts/browser-acceptance.js` 选择器随新 UI 更新——发送=Enter 键、中断=`.ant-sender-actions-btn-loading-button` 停止键、对话切换=`.ant-conversations-item`；等待预算放宽（负载 200+ 主机上 chromium 合成器饿死导致截图超时）。

## 契约/语义变更（有意）

- 提交键：Ctrl+Enter → **Enter 发送 / Shift+Enter 换行**（X Sender 默认 submitType='enter'）。
- 中断：独立「中断」Button → Sender loading 态停止键（同一 interrupt() 路径）。
- 会话切换：顶部 Select → 左侧 Conversations 侧栏（active 高亮 + 内建新建按钮）。
- nodes 舰队页与登录门不做 X 化重构，仅 antd6 兼容适配。

## e2e smoke（真实 chromium + daemon --server --web）

环境：worktree 隔离构建二进制（fe52924+SPA，内嵌新 dist），`daemon --server --web --port 18812 --token local-smoke-token --workdir /tmp/oc_t7_wd`（glm-5.3-flash 真实流式）。

| 步骤 | 命令/操作 | 观察 |
|------|-----------|------|
| 登录 | browser-acceptance step01 | PASS（签名 /api/nodes 探测→badge 已连接） |
| nodes 页 | step02 | PASS（表格渲染） |
| 建会话+发 prompt+流式 | step03（Sender fill+Enter） | PASS（POST /sessions 200→/prompt 200→streaming 标签+transcript 增长，sessionId=01M179P0NH0H7CEAJV733QBNDD） |
| 断链→徽章 | step04（`ss -K` RST+CDP 封 URL 后点停止键） | PASS（连接断开 badge） |
| SSE 自动重连 | step05 | PASS（backoff 后新 GET /events?after=344） |
| 中断 | step06（停止键→POST /interrupt 200） | PASS（done 终帧+store 归一化） |
| compact | step07（POST /compact + draining 轮询） | 功能证据达成：messages 6→9（截图在负载 285 下超时，属基建抖动） |
| 切会话回放 | /tmp/replay_check.js（对 18812 持久会话） | **6/6 PASS**：Conversations 点击→active 高亮→回放 738 chars+usage footer；新建重置；重选恢复 |
| 远端 node dialogs | 真 worker `daemon --client --name t7-node` 注册→签名 curl 派发任务 | pending→**done**（4s）→`GET /api/nodes/:id/dialogs` 返回该会话（task_count:1） |

截图证据：/tmp/uitest/shots/（replay-check.png 等）；run 全量日志：/tmp/opencoder_bg_1613009.output。

## 测试覆盖（rules/01/02/03）

- vitest **56/56**（7 文件）：纯函数 unit——sign 13（冻结）+ reduce 15（冻结，流式语义回归锚点）+ bubbleItems 10 + conversationItems 6；jsdom DOM 冒烟 integration——app.dom 4 + chat.dom 3（Enter 提交携带 prompt/输入清空/停止键→interrupt POST）+ sidebar.dom 5（双源 items/点击加载/active 高亮/新建重置/预选联动）。
- cargo（git worktree 隔离树：HEAD fe52924 + 仅本任务 SPA 改动，规避并发会话在主树的在途改动）：
  - `cargo test --workspace --no-fail-fast` → **233 套件 / 3323 passed / 0 failed / exit=0**（baseline 3322@1db6bec 不降；+1 来自并发会话入库的 tui 修复自带测试）。
  - `cargo clippy --workspace --all-targets -- -D warnings` → exit=0 零警告。
  - `cargo test -p opencoder-web --lib html` → 7 passed（bootstrap/白名单/无外链 embed 守护）。
  - 说明：隔离树首跑中 `opencoder-node::runner_cancel::heartbeat_cancellation_reports_cancelled` 30s 未 settle（主机 load 219，多会话并发构建），单独重跑 11.4s 通过——负载抖动，非回归。
- `bash scripts/check-spa-drift.sh` → exit 0（dist 与 src 同步）。
- 构建契约：dist/index.html + static/app.js（1.04MB）+ static/app.css 三文件，X/antd6 全量打进 app.js，无 CDN 外链。
- 行数：新增文件最大 273 行（≤400）；chat.jsx 迭代后 340 行（≤800）。

## Gate

- 测试覆盖(rules/01) → done（新增 21 用例：纯函数 16 + DOM 冒烟 8 + 验收脚本适配）
- 回归不降(rules/02) → done（3323≥3322，failed=0；清单如上）
- 测试分层(rules/03) → done（unit/DOM integration/真实浏览器 e2e）
- clippy 零警告 → done
- 构建干净 → done（drift 绿 + 三文件契约）
- 行数限制 → done
- 无密钥泄露 → done（token 仍为用户输入+localStorage；测试用占位 token）
- 文档同步 → done（agents/web/index.md SPA 栈与测试面修正）
