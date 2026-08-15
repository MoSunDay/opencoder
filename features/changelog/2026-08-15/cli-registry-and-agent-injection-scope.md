Commit: 366737aef3433255433c69620644e8b79a11f708

# `/cli` 注册管理与 CLI/MCP Agent 注入范围

- 新增 `/cli` 弹窗，可新增、编辑、删除、启停 CLI 注册；每项保存自由文本 `content`，启用后自动加入 system prompt 的 `Registered CLI` 段。
- CLI 与 MCP 每个注册项新增 `inject_to`：`parent`、`subagents`、`all`。旧配置缺省为 `parent`，保持原 MCP 仅父 Agent 可见的行为。（后续迭代已升级为 parent/explore/build 多选，见 `inject-to-multiselect-and-content-editor.md`。）
- MCP 的注入范围同时约束 system prompt 和对应 `mcp__...` 工具，选择 `subagents` 时仅 `explore`/`build` 可见，选择 `all` 时父 Agent 与子 Agent均可见；`workflow` 调度 Agent 仍不获得执行工具。
- `/cli` 与 `/mcp` 列表直接展示注入范围；编辑表单切换范围的方式已改为勾选对话框（见后续迭代）。配置保存后立即 reload，下一次模型调用生效。

配置键示例：`cli.<name>.{enabled,inject_to,content}` 与 `mcp_servers.<name>.inject_to`。

按需求免测；完成 release 构建以验证生产二进制可生成。
