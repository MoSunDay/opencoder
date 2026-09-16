Commit: eb6ca791cd8b766a4e51d8f91e26b9b1289775c7

# TODO 执行记录以父 Agent Say 为入口

## 行为

- 打开运行记录默认查看父 Agent 的 Say，复用 Agent 对话的 Steps、思考和工具调用展开层级；结构化决策和候选结果显示可读正文，原始回复与输入仍可展开。
- 固定 TODO 清单支持搜索与任务跳转，任务提供结果、要求、依赖跳转和历史会话选择。父 Agent 与任务在同一区域切换，保留滚动位置、展开状态及固定历史选择。
- 已访问会话按游标增量读取，隐藏时暂停轮询；初始分页只有输入时继续读取到 Say。分页停滞和读取失败明确报错，保留已经读取的内容，迟到响应不能污染另一会话。
- 「原始记录」继续提供冻结定义、逐次派发上下文、结果和事件文件；TODO 运行页与执行详情复用同一工作台。窄屏将控制按钮收进「运行操作」。

## 收口验证

- 固定已合并候选 `eb6ca791`，在 `/tmp/opencoder-todo-closure-check` 独立检出并重建 SPA、Server 和 Agent，防止共享工作区的后续迭代混入验收。
- SPA 全量：101 个测试文件、712 项通过，日志 `/tmp/opencoder-todo-closure-candidate-spa-tests.log`；SPA 构建通过，日志 `/tmp/opencoder-todo-closure-candidate-spa-build.log`。
- `cargo test --workspace`：5,250 项通过、0 失败、6 项既有忽略，日志 `/tmp/rel-cargo-test.log`。候选的 1,371 个 Rust 源文件与 Cargo 配置文件均与全量回归通过时一致，校验清单 `/tmp/opencoder-todo-closure-rust-source.json`。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 与 `cargo build --workspace --bins` 通过，日志 `/tmp/opencoder-todo-closure-clippy.log`、`/tmp/opencoder-todo-closure-build.log`。固定候选的 Server、Agent 独立重建通过，日志 `/tmp/opencoder-todo-closure-candidate-build.log`。
- Chromium 使用真实 Server、Agent、Review API 和独立数据存储；只有模型回复使用确定性夹具，断线用例明确注入 503。覆盖 16 项场景：目录和文件右键增删改名、非法文件拦截、自动校验后保存、旧版本不变及刷新回读、父 Say 默认入口、三次往返保留滚动与展开、上下文文件、任务重跑、历史会话固定、原始记录往返、断线恢复、Agent 重启后历史保留及 390px 窄屏。
- 回执 `/tmp/opencoder-todo-workbench-YIiT5i/result.json`：`PASS`，运行时错误为零，包含源码指纹、Server 实际返回的 JS/CSS 摘要及真实写入路由。验收开始和结束均校验 Server 产物与构建文件一致，且验收期间 SPA 源码未变。
- 可复现入口：构建 SPA 及候选二进制后，运行 `PLATFORM_BIN_DIR=<候选二进制目录> node scripts/acceptance/todo_workbench/main.js`。已有模拟接口用例继续覆盖嵌套工具详情、分页与不重复从零读取，回执 `/tmp/opencoder-todo-conversation-browser/result.json`。
- 此回执证明候选版本的开发与真实接口验收完成，不表示执行了生产发布；其他并行迭代需要各自验证。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 父 Say 默认入口、原位切换、位置与展开保留 | `starts with parent Say, switches in place and retains expanded steps and independent scroll positions` | `crates/web/spa/src/todo/review/conversation/workspace.dom.test.jsx` |
| 可见会话增量轮询与失败后保留内容 | `polls only the visible conversation with its own cursor and preserves cached Say after failures` | 同上 |
| 快捷键只收起当前会话 | `keeps inactive parent disclosure open when the current TODO is collapsed with the keyboard` | 同上 |
| 未执行任务、依赖跳转与搜索保留 | `opens waiting tasks and follows dependencies without losing the TODO filter` | 同上 |
| 固定历史会话与事件回看 | `preserves a chosen historical session as a new run appears and exposes its execution events` | 同上 |
| 跨输入分页、原文保留 | `continues past prompt-only pages to the first Say and keeps the original decision available` | `crates/web/spa/src/todo/review/conversation/session.dom.test.jsx` |
| 分页停滞与原游标重试 | `reports stalled pagination without looping and retries from the last valid cursor` | 同上 |
| 迟到响应隔离 | `does not carry a late response across keyed session changes` | 同上 |
| 结构化 Say 投影、历史会话顺序 | `renders structured decisions and results while preserving the exact original reply`、`keeps historical sessions in order and uses the last session after execution ends` | `crates/web/spa/src/todo/review/conversation/model.test.js` |
| 执行详情默认对话与只读原始记录 | `todos 明细默认展示父 Agent 与清单，并保留只读原始记录` | `crates/web/spa/src/fleet/detail/embeds.dom.test.jsx` |
| 真实编辑保存回读、会话切换与重跑历史 | `verifyEditor`、`verifyConversation`、`verifyHistory` | `scripts/acceptance/todo_workbench/{editor,conversation,main}.js` |

相关：[TODO 功能](../../todos/index.md)、[Web 模块](../../../agents/web/index.md)、[操作说明](../../../docs/todo-workbench.md)。
