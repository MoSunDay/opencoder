---
name: opencoder-projects
description: 使用 opencoder-cli 管理 OpenCoder 项目、专项和 TODO，读取关联执行的 Session、消息、过程和上下文，维护归属、看板、人员、标签及概述。TODO 工作流执行使用另一组命令，不能混用 ID。
---

# OpenCoder 项目管理

使用 `opencoder-cli project` 调用现有 Server API，不直接写数据库。项目在 CLI 中称为 `goals`，专项称为 `initiatives`，任务称为 `project todos`。

## 连接

先检查所用 CLI 的 `--help` 和连接配置，本环境可用 `opencoder-79 --help`。用户指定的服务器优先；当前 10.199.79.43 使用 `http://10.199.79.43:18081`。可使用已配置的 `--client-config /root/.config/opencoder/ctl-79.json`。

本环境已安装 `opencoder-79` 连接快捷命令，可直接用 `opencoder-79 project goals list`。它调用原生 CLI，固定连接 43，并仅为该进程将目标 IP 加入代理绕过名单。其他服务器使用原生 CLI；需要重新安装快捷命令时，将 [scripts/opencoder-79](scripts/opencoder-79) 安装到 PATH，并准备连接文件。

本机新版原生 CLI 位于 `/usr/local/lib/opencoder-cli/current/opencoder-cli`；43 上的 `/usr/local/bin/opencoder-cli` 已随发布更新。以下 `opencoder-cli` 示例指对应新版客户端；在本机优先用 `opencoder-79`。

```sh
opencoder-cli --server http://10.199.79.43:18081 --no-auth raw call GET /api/auth/status
opencoder-cli --server http://10.199.79.43:18081 --no-auth project overview
```

`/api/auth/status` 无需登录，返回 `enabled`。关闭时使用 `--no-auth` 或连接文件中的 `"auth_enabled":false`；开启时使用既有 `--token-file` 或环境变量 `OPENCODER_SERVER_TOKEN`，不创建、轮换或过期任何 Token。不要把凭证写进 Skill、Git、命令示例或报告。

若返回 401/403，核对连接和已授权身份；`--no-auth` 只省略请求头，不能关闭服务端鉴权。

## 操作

普通增删改查、字段和可直接使用的示例见 [references/crud.md](references/crud.md)。先读相关对象，按 ID 更新或删除；同名对象不假定唯一。长 Markdown 与中文正文写入临时 JSON 文件后使用 `--json @/absolute/path.json`，避免 shell 转义出错。

- 项目 → 专项 → TODO；专项的 `goal_id`、TODO 的 `initiative_id` 都可为空。TODO 通过专项归属项目。
- PATCH 只发送要改的字段；人员清空使用 `owner_user_name:null`、`follower_user_names:[]`；解除归属使用对应 ID 字段的 `null`。负责人和关注人先用 `project people` 查询现有用户名。
- TODO 看板仅接受 `backlog`、`todo`、`in_progress`、`done`。执行结束不会自动完成 TODO；只按用户要求或已授权验收更新看板。专项没有自身状态。
- 删除项目保留专项与 TODO，专项解除项目归属；含 TODO 的专项拒绝删除。若用户只要求删除专项，不擅自删除其 TODO；可以在已授权范围内先移动或解除 TODO 归属。删除 TODO 不取消其关联的原生执行。
- 本 Skill 的 CRUD 不发起执行。明确要求派发、关联或继续能力时，查 `project todos dispatch/attach/prompt --help`，使用真实能力和稳定执行 ID；项目 TODO 与 `opencoder-cli todo` 的工作流任务是不同对象。
- 概述按需用 `project initiatives summary-context ID`、`project goals summary-context ID` 收集证据，通过 PATCH 的 `current_summary` 保存。证据读取失败明确报告来源，不视为已完成验收。
- 了解 TODO 的执行上下文时，用 `project todos context TODO_ID` 读取任务、全部评论和关联执行的原生详情、消息、事件、完整字段；历史较多时使用 `--execution EXECUTION_ID` 只读取指定关联。`assignments` 保留全部关联，`executions` 是本次读取的证据。`complete:false` 与 `issues` 表示证据缺失，退出码 4，不能当作完整上下文；运行中仍可能产生新消息。轻量查看用 `links`、`result`、`messages`，分页用返回的 `next_cursor.seq/offset` 传给 `--seq/--offset`。具体示例见参考文档。

命令成功后读取对象验证关键字段；删除后确认 GET 返回 404 或列表已移除。只重试可确认结果的请求；创建超时先按已创建 ID、名称和时间查重。输出用户关心的名称、ID、归属、状态与实际结果。CLI 返回 0 表示请求成功；1 为连接或配置错误，2 为 401/403，4 为请求拒绝或 context 的证据读取不完整，不将局部成功写成全部成功。
