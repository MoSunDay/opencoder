Commit: 8ed37a1aef29f864c8a7ad302e04766c38e5c627

# Agent 执行详情统一 Say / Step 渲染

## 问题与行为

全部执行抽屉原先逐条提取消息的 text 块，工具消息显示为 `—`，reasoning 与 tool_use / tool_result 无法组成 TUI 的步骤层级。现在执行详情与会话交互共用 transcript reducer 和组件：每个 Say 关闭自己的 Step 组，工具结果按 call ID 回填，回答正文始终可见，步骤与调用逐层展开。

完成后的 Say 按 Markdown 渲染后的首个非空行生成标题，并从同一行表示中去重正文；流式输出仍显示原始行。标题、粗体、列表、代码围栏及多行内容不会因为删除原始首行而丢失结构。展开箭头统一为闭合 `▸` / 展开 `❯`，不再叠加两套箭头；image 展示标记不会误生成 Say 标题。

执行事件订阅支持同一执行的多轮历史，历史 done / error 是轮次边界，EOF 才结束该次订阅；续写与重连从已处理的序号继续。原有单轮订阅的终态语义保持不变。实时记录和持久化消息分别进入同一 reducer / renderer，避免把快照与其历史事件重复叠加。状态提示不参与后续步骤的稳定 key 编号，快照刷新保留展开状态。执行抽屉宽度受视口限制，窄屏可查看完整内容。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 跨消息 Say/Step 配对、思考/工具结果展开与全部收起 | `pairs separate persisted messages and retains nested disclosure on refresh` | `crates/web/spa/src/fleet/detail/transcript.dom.test.jsx` |
| 实时状态行消失后保留展开状态 | `retains an expanded ladder when live status rows disappear on snapshot settlement` | 同上 |
| image 不冒充 Say | `does not turn presentation-only image blocks into a Say heading` | 同上 |
| Markdown 首行去重、流式切换、代码围栏、列表、HTML | `TUI Say preview and body use the same rendered lines` | `crates/web/spa/src/transcript/markdown.test.js` |
| 多轮 done/error、续写与重复序号去重 | `execution replay and continuation` | `crates/web/spa/src/fleet/detail/transcript.test.js` |
| 追平事件水位、续写游标、切换执行与离线错误 | `execution transcript subscription lifecycle` | `crates/web/spa/src/fleet/detail/liveTranscript.dom.test.jsx` |
| 历史终态后继续回放与单轮终态兼容 | `fleet execution event history` | `crates/web/spa/src/sse.history.test.js` |
| 实际贪吃蛇执行记录、真实 bash 续写、Markdown、桌面与 390px 视口 | Chromium + 正式 Server / Node / 模型 | `/data00/opencoder-delivery/20260907-say-step-parity/` |

验收结果：

- SPA：`npm test` → 47 个文件、416 个测试通过；构建和 `scripts/check-spa-drift.sh` 通过。
- Rust：`cargo test --workspace --locked -- --test-threads=4` → 4,752 passed / 0 failed / 5 个既有手动 ignored；原始输出为上述目录中的 `rust-tests-verified.log`。本机测试使用 `TMPDIR=/var/tmp`，并在 `NO_PROXY` / `no_proxy` 中包含回环地址，避免磁盘容量门槛与环境代理干扰测试节点。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 零警告；`cargo build --workspace --locked` 成功。
- 提交 `8ed37a1` 已更新到本机 Server / Node，节点 ID 保持不变，接单状态为 Ready。当前 Web 的真实 bash 续写、历史 20 组 Say/Step、Markdown 去重、工具展开和 390px 视口均通过；返回的全部前端文件与发布包目录哈希一致。部署与浏览器证据分别为 `deployment.json`、`deployed-browser.json`、`final-state.json`。

## 相关文档

- [Web 模块](../../../agents/web/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
