Commit: 1ba83e9ec97a38106af1bbe309860a3e5276cc4f

# 项目 CLI 与鉴权开关

`opencoder-cli project` 提供现有项目接口的增删改查，项目使用 `goals`，专项使用 `initiatives`，任务使用 `project todos`。`opencoder-cli todo` 是独立工作流能力，不是项目 TODO。

## 当前服务器

10.199.79.43 的公共入口为 `http://10.199.79.43:18081`。先读取开关，再执行项目命令：

本环境已有快捷命令 `opencoder-79 project goals list`、`opencoder-79 project initiatives list`、`opencoder-79 project todos list`。它使用同一原生 CLI 和单独连接文件，并将 43 的 IP 加入本进程代理绕过名单。后续原生 CLI 示例也可用此快捷命令简化；其他服务器保留各自配置。

本机提供的新版原生 CLI 为 `/usr/local/lib/opencoder-cli/current/opencoder-cli`；43 上使用 `/usr/local/bin/opencoder-cli`。下面的 `opencoder-cli` 命令指对应新版客户端，本机使用快捷命令最方便。

```sh
opencoder-cli --server http://10.199.79.43:18081 --no-auth raw call GET /api/auth/status
opencoder-cli --server http://10.199.79.43:18081 --no-auth project goals list
opencoder-cli --server http://10.199.79.43:18081 --no-auth project initiatives list
opencoder-cli --server http://10.199.79.43:18081 --no-auth project todos list
```

`--no-auth` 省略 Authorization，不读取已有 Token 文件；它只控制 CLI 请求。服务端仍开启鉴权时，项目命令返回 401。stdout 成功时为 JSON；stderr 错误包含实际 HTTP 状态，退出码 1 是连接/配置错误、2 是 401/403、4 是其他业务拒绝。

可以保存单独的连接文件，避免覆盖其他 Server 的默认连接：

```json
{"server":"http://10.199.79.43:18081","auth_enabled":false}
```

用 `opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project goals list` 调用。CLI 默认仍开启鉴权，原有 Token 配置继续可用。显式 `--token` / `--token-file` 优先于文件中的免鉴权设置；与 `--no-auth` 互斥。

## 增删改查

每类都提供 `list`、`get ID`、`create --json BODY`、`patch ID --json BODY` 和 `delete ID`。长正文支持 `--json @/absolute/file.json`。成功创建返回对象 ID，后续按 ID 操作；名称不保证唯一。

```sh
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project goals create --json '{"title":"示例项目"}'
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project initiatives create --json '{"title":"示例子项","goal_id":"PROJECT_ID"}'
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project todos create --json '{"title":"示例任务","draft":"记录执行步骤及证据","initiative_id":"INITIATIVE_ID"}'
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project todos patch TODO_ID --json '{"board_status":"in_progress"}'
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project todos get TODO_ID
opencoder-cli --client-config /root/.config/opencoder/ctl-79.json project todos delete TODO_ID
```

完整字段、三层创建/修改/删除示例、人员、标签、评论和概述见 [Skill 参考](../skills/opencoder-projects/references/crud.md)。专项和 TODO 均可独立存在，解除归属传对应 ID 字段的 `null`。含 TODO 的专项拒绝删除；删除项目仅解除专项归属；删除 TODO 不取消原执行。CRUD 不自动派发或验收任务。

## 读取 TODO 的执行详情和上下文

```sh
opencoder-79 project todos context TODO_ID
opencoder-79 project todos context TODO_ID --execution EXECUTION_ID > /tmp/todo-context.json
opencoder-79 project todos links TODO_ID
opencoder-79 project todos result TODO_ID --execution EXECUTION_ID
opencoder-79 project todos messages TODO_ID --execution EXECUTION_ID --seq 7 --offset 1024
```

`context` 从 TODO 的关联读取原生执行，默认包含全部历史，也可用 `--execution` 选择其中一次。输出包含任务、全部评论、执行归属、原始请求、Session 元数据、完整消息、事件、结论及省略字段的完整内容；Team 和 TODO 工作流另含讨论与工作项。大记录会自动翻页，原始数据仍由所属 Runtime 提供。没有执行时返回空证据，不创建会话或改变任务。

`complete:false` 时查看 `issues` 的失败来源；命令保留已读数据并返回 4。证据完整只指本次选择的读取范围，运行中可能产生新消息，也不表示任务已完成或验收。轻量读取用 `links`、`result` 和 `messages`，后两者可指定历史执行；`messages` 的原生分段游标来自返回的 `next_cursor.seq/offset`，旧参数 `--after-seq/--message-offset` 继续可用。

网页点击 TODO 执行 ID 在新页面定位该原生执行及其能力过程；Agent/Operator 展示原 Session 消息，其余能力展示各自的讨论、步骤、工作流或调度记录。执行 ID 是唯一必需的定位输入，实际类型和 Runtime 来自 Server；不受列表第一页、空类型或保存的导航页影响，原 TODO 草稿保留。节点离线显示实际读取错误。已有执行 ID 也可直接用 `opencoder-79 exec get EXECUTION_ID` 读取原生明细。

## 服务端开关

单个 Server 使用 `--auth-enabled true|false`，默认 true；启动凭据仍用于机器通道和独立资源服务。部署配置使用 JSON 布尔字段：

```json
{"deployment":{"auth_enabled":false}}
```

将字段加入现有 `/data00/opencoder-platform/deployment.json`，保留其余配置和凭据文件；开关随新 Server 实例平滑发布生效，不热重启已有 Runtime。恢复时设为 true，并在后续平滑发布中生效；已有用户和 Token 无需重建。`GET /api/auth/status` 是当前实例实际状态，配置文件本身不代表已生效。

关闭时所有公共 API 请求使用固定 admin 身份，包括 TODO 评论署名；原 Token 不决定角色。重新开启后恢复原 Token 校验与角色权限。独立资源服务、Host、Runtime 的机器鉴权保留，Server 代转资源请求时仍使用原机器凭据。

## Skill

[opencoder-projects](../skills/opencoder-projects/SKILL.md) 是可安装的 Codex Skill，安装目录为 `~/.codex/skills/opencoder-projects`；也可将同一目录安装到 OpenCoder 的 `~/.opencoder/skills`。使用 `$opencoder-projects` 发起项目操作。Skill 使用 CLI，不写数据库或修改鉴权数据。
