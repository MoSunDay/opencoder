Commit: fdd5cbe93b027549c05e3616c4feebcf437d5fc0

# TODO 执行 Session 与 CLI 上下文

TODO 只保存泛化执行 ID；打开详情必须由 Server 按 ID 读取原能力过程。原页面只在初次挂载选择执行页，随后恢复主页状态或改变地址可能丢失定位；项目还要求先读到执行类型才生成链接。旧版 Chromium 的详情渲染另受 `Array.at` 和 `URLSearchParams.size` 影响。原 CLI 消息命令把分页参数发成服务端不识别的名称，可能重复读取第一页。

## 行为

- 执行地址持续优先于保存的主页选择，支持地址变化、浏览器历史和刷新；主动切换菜单或关闭详情清除定位。新链接只携带执行 ID，放在地址片段中以保留重定向后的定位，兼容旧查询参数。TODO 不要求类型或当前索引可读；Server 返回实际类型后展示 Agent/Operator 会话、Team 讨论、DAG 步骤、TODO 工作流或 Brain 调度。执行总表点击 ID 同步地址。
- 原生执行详情和消息翻页去除上述浏览器 API 依赖。会话 Markdown 解析失败时显示普通文本，保留执行 ID、所属节点与消息。执行 ID 仍在新页面打开，原任务与评论草稿保留。
- 新增 `project todos context TODO_ID [--execution EXECUTION_ID]`，读取任务、全部评论和关联执行的原生详情、Session、完整消息、事件与省略字段。默认全部历史；指定执行必须已经关联，不依赖当前能力选择。不创建、继续执行或改写看板。
- 部分证据失败保留已读内容，输出 `complete:false`、`issues` 并返回 4；空 TODO 正常返回空执行。完整证据不表示任务验收通过。
- `project todos messages --seq/--offset` 使用原生游标参数；保留 `--after-seq/--message-offset` 旧别名。同步 CLI 用法和 opencoder-projects skill。
- CLI 上下文优先采用原生详情中的执行类型，过期或空关联提示不会跳过实际消息；轻量 messages 也直接按 ID 请求 Server，由 Server 判定能力是否支持读取。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 执行地址不被主页恢复覆盖、挂载后到达地址 | `keeps an ID-only execution route ahead of later homepage state restoration`、`opens a generic execution ID when navigation arrives after the homepage mounted` | [appNavigation.dom.test.jsx](../../../crates/web/spa/src/appNavigation.dom.test.jsx) |
| 原生 Team、DAG、TODO、Brain 过程按泛化 ID 读取 | `resolves a generic ID to its native ... process with no type in the link` | [records.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/records.dom.test.jsx) |
| 无列表依赖、地址变化、历史和总表点击 | `follows ID-only links, hash changes and browser history without relying on the list`、`keeps execution list clicks in the address so refresh opens the same record` | [links.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/links.dom.test.jsx) |
| 历史 Agent/Operator Session、旧浏览器和翻页 | `opens the actual linked ... Session and pages messages in older browsers` | [session.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/session.dom.test.jsx) |
| 原 Runtime 离线错误 | `keeps the linked execution visible and reports its original Runtime being offline` | [session.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/session.dom.test.jsx) |
| Markdown 兼容回退 | `retains readable Session text when the Markdown parser lacks browser APIs` | [markdown.test.js](../../../crates/web/spa/src/transcript/markdown.test.js) |
| 全历史、分段消息、事件正文、离线来源 | `context_reads_all_history_without_needing_a_current_capability_and_exposes_missing_evidence` | [project_context.rs](../../../crates/ctl/tests/project_context.rs) |
| 指定历史、拒绝外部执行和空 TODO | `context_selects_only_linked_history_and_keeps_empty_todos_readable` | [project_context.rs](../../../crates/ctl/tests/project_context.rs) |
| 真实 CLI 进程、JSON、部分成功与 HTTP 退出码 | `context_cli_dispatches_prints_json_and_preserves_partial_and_http_exit_codes` | [project_context.rs](../../../crates/ctl/tests/project_context.rs) |
| CLI 消息游标和原生执行一致 | `project_conversation::todo_input_and_reads_use_the_same_native_execution` | [conversation.rs](../../../crates/ctl/tests/project/conversation.rs) |
| 空类型仍请求原生消息、由 Server 拒绝不支持的能力 | `project_conversation::todo_messages_use_native_id_even_when_reference_type_is_missing` | [conversation.rs](../../../crates/ctl/tests/project/conversation.rs) |

- 首轮 SPA 六个文件 37 passed；导航补齐后按修改重新验证 navigation/session、navigation/links、navigation/records、project/todosTab、appNavigation、navPersistence、executionControls，当前共 41 例通过。CLI 最新定向范围为 context 三例、summary 五例和 server_local 的 project_conversation 两例，10 例通过，覆盖空或过期类型提示。首轮 CLI 进程测试的错误输出断言失败记录保留，修正后通过。
- 真实只读验证：线上 TODO 的历史 Operator Session，220 条消息、226 条事件，`complete:true`。候选 SPA 在 Chromium 90 和新版 Chromium 实际点击 TODO 执行链接，核对详情 ID、Session ID、原 Runtime 与原抽屉。
- 导航候选浏览器验证覆盖桌面与 390 像素页面，实际点击、刷新、地址切换历史执行和未保存标题保留。线上 Agent、Operator、DAG、TODO 工作流、Brain 的现有记录均以仅含 ID 的地址读取原生详情；线上无 Team 记录，Team 使用确定性原生 API 数据验证完整组件。
- `cargo clippy -p opencoder-cli --all-targets -- -D warnings`：零警告；SPA 构建、Skill 校验、格式与差异空白检查通过。
- 日志与浏览器回执：`/tmp/opencoder-todo-session-20261015`、`/tmp/opencoder-todo-navigation-20261015`。测试范围为本次修改及直接影响链路，未运行全量测试。

## 发布与线上验证

- 最终代码 `fdd5cbe93b027549c05e3616c4feebcf437d5fc0` 已通过公共入口的信号发布完成；干净工作树构建的七个程序与 SHA256SUMS、build-info、SPA 漂移检查一致。当前线上 JS 与候选产物逐字节摘要一致。
- 桌面和 390 像素页面实际点击 TODO 执行 ID，通过刷新、切换历史执行及保留未保存标题验证；旧查询参数链接也可读取原 Session，关闭后清除定位。五种线上现有能力记录均显示对应原生过程；Team 使用确定性 API 数据测试，不新建线上执行。
- 已安装 CLI 默认读取该 TODO 的全部两次关联，得到 304 条消息和 315 条事件，`complete:true`；指定历史执行单独读取 220 条消息和 226 条事件。两台机器的 CLI 和 Codex/OpenCoder skill 均已更新。
- 900.000081 秒项目及原 Session 读取观察通过，1062 次请求零失败；最大单次请求 2.008061 秒，最大采样间隔 7.785106 秒。此观察验证项目与执行读取，不作为全量调度或模型质量验收。
- 六条样本执行的原 Runtime 归属保持，切换时旧 Runtime、独立资源服务和 rpcbind 进程未变；已有用户和 Token 表的只读摘要保持，鉴权仍关闭。退役版本 `4c174685` 保留两条执行记录，不等待其结束。
- 线上回执：`/data00/opencoder-delivery/todo-navigation-20261015`；本机汇总：`/tmp/opencoder-todo-navigation-20261015/delivery-summary.json`。原工作区未提交的执行隔离与产物改动保留，本次发布未包含它们。

相关说明：[项目工作台](../../project/index.md)、[CLI 用法](../../../docs/project-cli.md)、[项目操作 skill](../../../skills/opencoder-projects/SKILL.md)。
