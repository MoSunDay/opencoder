# TODO 目录工作台

TODO 是由父 Agent `workflow` 驱动的 Agent Loop。父 Agent 决定任务派发、验收和后续操作；每个 TODO 获得完整的需求背景、执行要求、验收标准和已接受的依赖结果，独立执行并返回结果。父 Agent 保持简洁的上下文，过程记录用于 Review、恢复和从指定任务重跑。

## 定义目录

模板版本是实际的文件目录：

```text
todo/<模板名>/
  todo.json
  v1/
    workflow.json
    objective.md
    env.json
    todos/<任务 ID>/
      task.json
      context.md
      instructions.md
      acceptance.md
```

| 文件 | 内容 |
| --- | --- |
| todo.json | 模板名称、说明、当前版本和版本列表，由版本管理维护 |
| workflow.json | `schema_version: 1`、`id`、`name`、`constraints`、`todos` ID 数组、`metadata` |
| objective.md | 工作流目标 |
| env.json | `{"env": null}` 或绑定的环境名称 |
| task.json | `title`、`agent`、`max_attempts`、`depends_on`、`required_tool_calls`、`metadata` |
| context.md | 该 TODO 的需求背景 |
| instructions.md | 该 TODO 的执行要求 |
| acceptance.md | 该 TODO 的验收标准 |

目录名是任务 ID。`workflow.json.todos` 维护任务清单；父 Agent 根据依赖和状态调度。正文必须非空；任务 ID、依赖、Agent、重试次数和工具验收规则遵循 TODO 框架校验。未声明的文件、未知配置字段、非法 JSON、缺失文件和无效环境绑定均报错。

## 编辑与校验

左侧搜索和选择文件，右侧使用 CodeMirror 编辑 JSON 和 Markdown，支持行号、高亮、查找、缩进、撤销、JSON 格式化、Markdown 预览和分屏。切换文件保留草稿、光标和撤销历史。`Ctrl/Cmd+S` 保存新版本。

新增或复制 TODO 会生成完整任务目录。重命名会同步修改依赖；存在依赖引用时禁止删除任务。可以移除多余文件，点击缺失文件的报错位置补写必需内容。

加载不合规文件时弹窗报错。编辑期间显示行内诊断；校验、保存和运行前检查发现错误时，弹窗列出文件路径、行列与原因，点击可定位。校验失败阻止发布和运行，并保留草稿供修复。

保存创建新的完整版本并切换当前版本。服务器校验全部文件和环境引用；模板文件锁与元数据修订检查避免并发覆盖。版本目录先完整写入，再发布当前版本。已存在版本不可原地改写。旧 `context.json` 模板可读取，编辑后保存为新目录版本，旧文件保持原样。

## 执行与 Review

Server 从定义目录加载工作流，解析并冻结环境配置后派发。Node 将冻结定义写入执行目录下的 `definition/` 并从目录加载；恢复和重跑也检查目录与冻结工作流一致。

运行工作台提供只读 `definition/` 和 `process/` 文件视图。`process/` 从持久化状态和事件投影，包括父 Agent 决策、任务状态、每次派发的完整上下文、结果及会话引用。每次派发按事件序号单独归档，历史重跑不会覆盖旧尝试。较早记录按需分页，较大内容分段读取，点击会话文件可以打开会话 Review。

选择任务后可“从选中任务重跑”：沿用该次运行冻结的定义，重置目标任务及其依赖后继，保留独立分支、历史会话、文件和外部操作结果。修改模板定义后要发起新运行。节点离线或状态陈旧时明确显示错误，并禁用依赖有效状态的控制操作。

## 接口与 CLI

- `GET /api/todo/templates/:name/:version/files` 返回文件文本、目录条目、修订标识和诊断。`?tree=true` 只返回目录；`?path=...` 读取单个文件。
- `POST /api/todo/validate-files` 接受 `{files: {"相对路径": "文件内容"}}`，校验失败返回 `diagnostics`。
- 创建模板接受 `{name, files}`；`POST .../:name/new-version` 接受 `{source_version, expected_revision, files}`。旧调用者也可以提交 `spec` 或 `binding`，编辑时必须携带修订标识或 `expected_current`。
- 原 `GET .../context.json` 返回组装后的工作流；原 PUT context/env 接口返回 409，引导保存新版本。
- `GET /api/todo/workflows/:id/review?section=files` 返回运行冻结定义的文件视图，复用 Review 分段传输。
- 本地 `todos validate`、`todos run` 接受目录路径，也保留单个 WorkflowSpec JSON 输入；目录输入会解析环境绑定。
