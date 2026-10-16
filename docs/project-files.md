# TODO 关键信息与任务文件

TODO 的“关键信息”用于持续记录目标、进展、结论、卡点、下一步和资料位置，方便交接。
正文继续使用 Markdown，API/CLI 字段仍是 `draft`；不新增正文历史，也不自动改写任务内容。

TODO 详情的“文件”页签支持上传、刷新、单文件下载和确认删除。文件与关键信息、评论分别提交，
切换页签保留草稿。新建任务先保存，再上传文件。viewer 可读取和下载，editor/admin 可上传和删除。
文件读取失败显示实际错误，不作为空列表处理。有文件或无法检查文件的 TODO 不能删除，须先明确删除文件。

## 目录与配置

文件目录为 `<files_dir>/<YYYY-MM-DD>/<todo_id>/<相对路径>`。日期取 TODO 创建时间的 **UTC 日期**，
跨天继续、修改标题和调整归属不会改变目录。已有 TODO 使用其原创建时间，首次上传才创建任务目录。
仅保存用户或 Agent 主动上传的资料；原生执行记录和工作区仍在执行节点，不启动后台收集。

```json
{
  "project": {
    "todo_storage": {
      "files_dir": "/data/opencoder/exports/todos",
      "max_file_bytes": 536870912,
      "uploads_enabled": true,
      "nfs_service_url": "http://127.0.0.1:18085",
      "nfs": { "enabled": true, "host": "127.0.0.1", "port": 2053 }
    }
  }
}
```

默认单文件上限为 512 MiB。独立启动未指定根目录时使用工作目录数据域的 `project/todo-files`；
基础数据目录先解析到实际位置，兼容将其迁移到其他磁盘的既有配置。
部署工具管理为 `<deployment.state_dir>/exports/todos`。默认不启动 NFS，监听地址默认 `127.0.0.1`，
端口默认 2053；需要远端挂载时按部署网络配置监听地址。导出始终只读，不接受 `read_only` 配置。

根目录必须是绝对路径，不得使用符号链接，不得与其他资源根、Server 数据目录互相包含。
其同级 `<files_dir>.state` 保存上传临时文件和文件锁，与根目录必须位于同一文件系统；
如果单独挂载磁盘，应在挂载点下再选择一个子目录作为根目录。`.state` 不通过 NFS 导出。
Server 在自身数据目录的 `todo-storage.json` 固定实际根目录；改变配置后不静默切换。
需要迁移时应停止写入，完整迁移文件及协调目录，核对后同步 Server 与专用服务使用的同一绑定文件和配置。

受管部署由 `opencoder-todo-resources.service` 专门承担 TODO NFS，使用 `--todo-resources` 启动，
管理接口监听回环地址 18085，NFS 默认对内网监听 2053。它与现有资源服务分别运行，
普通应用发布不重启任一 NFS 服务。未配置 `nfs_service_url` 的独立 Server 自行托管。Agent 配置页显示“TODO 文件只读导出”。
查询、启停使用 `GET/POST /api/project/todos/nfs`，POST 正文为 `{"enabled":true}` 或 `{"enabled":false}`。
临时启停不修改持久化配置。停止 NFS 不影响 HTTP 上传和下载。

```sh
mount -t nfs -o ro,vers=3,tcp,port=2053,mountport=2053,nolock server:/ /mnt/todo-files
```

首次发布先安装包含删除保护的兼容版本，上传保持关闭，再发布完整页面。
两版均具备 `todo_files_contract=1` 后，使用 `deploy.sh --todo-uploads enable` 开放上传；
命令先持久化最低回滚能力要求，再修改配置。`--todo-uploads disable` 关闭新上传并保留读取。
回滚保留专用服务及文件，并拒绝缺少保护的旧 Server；不恢复旧数据库。
首次安装中断可用同一发布包续跑；未开放上传且没有文件时，先回退仍依赖该服务的 Server，再用 `--restore-todo-service` 恢复配置与服务安装。
备份包含实际绑定的文件根、绑定及专用服务恢复记录，包括自定义根目录。
备份期间通过跨进程锁暂停文件操作；既有数据库备份的跨库一致性语义保持不变。

## API 与 CLI

| 接口 | 行为 |
|---|---|
| `GET /api/project/todos/:id/files` | 返回 `directory`、`max_file_bytes`、`files`；文件包含 `path`、`bytes`、毫秒时间 `modified_at` |
| `PUT /api/project/todos/:id/file?path=…` | 请求体为原始二进制，成功返回 201 和 `path`、`bytes` |
| `GET /api/project/todos/:id/file?path=…` | 下载单个文件，返回附件响应 |
| `DELETE /api/project/todos/:id/file?path=…` | 删除指定文件 |

路径为相对路径，支持子目录和中文，拒绝 `..`、绝对路径、空段、符号链接及特殊文件。
上传完成并持久化后才发布，失败或断连不发布半文件。同名返回 409，须换名或明确删除旧文件；不隐式覆盖。
上传关闭或专用管理服务不可达返回 503，管理请求总超时 10 秒，不自动重试写请求。
超过大小上限返回 413；任务或文件不存在返回 404，存储不可用返回错误并保留任务。
服务端和 CLI 以流方式传输，不使用 Base64。代理配置只对文件路由放开请求体限制，大小仍由 Server 检查。

```sh
opencoder-cli project todos files list TODO_ID
opencoder-cli project todos files upload TODO_ID ./notes.md --path handoff/notes.md
opencoder-cli project todos files download TODO_ID handoff/notes.md --output ./downloaded-notes.md
opencoder-cli project todos files delete TODO_ID handoff/notes.md
opencoder-cli project todos context TODO_ID
```

未指定上传路径时使用本地文件名。下载不覆盖已有本地文件，成功后才发布输出文件。
`context` 的 `record.files` 包含文件清单及每个文件的 `download_path`，不自动下载文件内容。
读取失败或清单响应不完整时列入 `issues`，返回 `complete:false` 和退出码 4；仍不代表任务验收结论。

相关：[项目 CLI](project-cli.md)、[项目工作台](../features/project/index.md)、[项目约定](../rules/07-project-module-contract.md)。
