# 增删改查与字段

以下示例适用于鉴权已关闭的 Server。请将占位地址替换为目标地址；代理设置按当前连接环境配置。

```sh
export OPENCODER_SERVER_URL=https://opencoder.example
opencoder-cli --no-auth project overview
opencoder-cli --no-auth project people
```

为避免每次加 `--no-auth`，连接文件可保存 `{"server":"https://opencoder.example","auth_enabled":false}`，用 `--client-config PATH` 指定。命令行 Token 参数可以覆盖文件中的免鉴权设置；`--no-auth` 与显式 Token 参数互斥。

## 三层对象

| 用户称呼 | 命令 | 创建正文 | 可修改字段 |
| --- | --- | --- | --- |
| 项目 | `project goals` | `title` 必填 | `title`、`detail_md`、`status`（active/archived）、`sort`、`current_summary`、人员 |
| 专项 | `project initiatives` | `title` 必填，`goal_id` 可选 | `title`、`detail_md`、`goal_id`、`sort`、`current_summary`、人员；不接受 `status` |
| TODO | `project todos` | `title`、`draft` 必填，`initiative_id` 可选 | `title`、`draft`、`initiative_id`、`board_status`、`position`、`tag_ids`、人员 |

三层人员字段均为 `owner_user_name` 与 `follower_user_names`，独立保存、不继承。创建时不需要构造 ID，成功正文中的 `id` 是后续操作依据。列表分别返回 `goals`、`initiatives`、`todos` 数组，GET 返回单个对象，PATCH/DELETE 返回 `ok`。

```sh
# 项目
opencoder-cli --no-auth project goals list
opencoder-cli --no-auth project goals create --json '{"title":"示例项目","detail_md":"记录项目目标"}'
opencoder-cli --no-auth project goals get PROJECT_ID
opencoder-cli --no-auth project goals patch PROJECT_ID --json '{"title":"更新后的示例项目"}'
opencoder-cli --no-auth project goals delete PROJECT_ID

# 专项：替换实际项目 ID；独立专项省略 goal_id
opencoder-cli --no-auth project initiatives list --goal PROJECT_ID
opencoder-cli --no-auth project initiatives create --json '{"title":"示例子项","goal_id":"PROJECT_ID"}'
opencoder-cli --no-auth project initiatives get INITIATIVE_ID
opencoder-cli --no-auth project initiatives patch INITIATIVE_ID --json '{"detail_md":"记录子项目标"}'
opencoder-cli --no-auth project initiatives patch INITIATIVE_ID --json '{"goal_id":null}'
opencoder-cli --no-auth project initiatives delete INITIATIVE_ID

# TODO：替换实际专项 ID；未归属任务省略 initiative_id
opencoder-cli --no-auth project todos list --initiative INITIATIVE_ID
opencoder-cli --no-auth project todos create --json '{"title":"示例任务","draft":"记录执行步骤及证据","initiative_id":"INITIATIVE_ID"}'
opencoder-cli --no-auth project todos get TODO_ID
opencoder-cli --no-auth project todos patch TODO_ID --json '{"board_status":"in_progress","draft":"补充分析要求"}'
opencoder-cli --no-auth project todos patch TODO_ID --json '{"initiative_id":null}'
opencoder-cli --no-auth project todos delete TODO_ID
```

不带 `--goal` 或 `--initiative` 的列表包括全部归属及未归属对象。列表不是同名对象的唯一性判断。

## 长正文、人员和标签

```sh
opencoder-cli --no-auth project todos patch TODO_ID --json @/tmp/todo-patch.json
opencoder-cli --no-auth project todos patch TODO_ID --json '{"owner_user_name":null,"follower_user_names":[]}'
opencoder-cli --no-auth project tags list
opencoder-cli --no-auth project tags create --json '{"scope_type":"global","scope_id":"","name":"优先处理"}'
opencoder-cli --no-auth project todos patch TODO_ID --json '{"tag_ids":["TAG_ID"]}'
```

Tag 可属于 global/project/initiative；只有当前 TODO 归属范围内的标签才能关联。先查询真实 ID，`tag_ids:[]` 清空选择，修改归属由服务端按同名定义重新匹配。

## 评论与概述

```sh
opencoder-cli --no-auth project todos comments list TODO_ID --limit 50
opencoder-cli --no-auth project todos comments add TODO_ID --json '{"request_id":"stable-note-id","body_md":"已记录分析证据"}'
# 先读取目标评论的 seq 和 revision，再提交修改。
opencoder-cli --no-auth project todos comments edit TODO_ID COMMENT_SEQ --json @/tmp/comment-edit.json
opencoder-cli --no-auth project todos comments delete TODO_ID COMMENT_SEQ --expected-revision CURRENT_REVISION
opencoder-cli --no-auth project initiatives summary-context INITIATIVE_ID
opencoder-cli --no-auth project initiatives patch INITIATIVE_ID --json @/tmp/initiative-summary.json
opencoder-cli --no-auth project goals summary-versions PROJECT_ID
opencoder-cli --no-auth project goals summary-restore PROJECT_ID VERSION
```

评论支持追加、编辑和删除。编辑 JSON 为 `{"body_md":"完整正文","expected_revision":1}`，版本取最新读取结果，不固定填 1；删除同样使用读取时的版本，过期返回 409。editor 只能修改或删除自己提交的评论，admin 可管理全部评论，viewer 只读。编辑保留编号、创建时间、署名和来源。追加重试复用 request_id 和原始完整正文，编辑后返回当前评论；删除后原追加请求返回冲突，不能重新创建。默认署名为实际提交身份，鉴权关闭时为 admin；能力署名使用真实注册 `capability_id`，可选 `execution_id` 须已关联当前 TODO。概述版本最多保留十条，恢复会新增版本，不改写旧版本。

同一轮 Context 先汇总再一次发布；补充已有内容时优先编辑原评论。用户要求合并时，先保存合并正文并回读，再删除被合并的评论；删除前核对 TODO、编号、正文和版本，保留原始快照。不得通过直接写数据库绕过接口。

## TODO 执行上下文

```sh
opencoder-cli --no-auth project todos links TODO_ID
opencoder-cli --no-auth project todos context TODO_ID
opencoder-cli --no-auth project todos context TODO_ID --execution EXECUTION_ID
opencoder-cli --no-auth exec get EXECUTION_ID
opencoder-cli --no-auth project todos result TODO_ID --execution EXECUTION_ID
opencoder-cli --no-auth project todos messages TODO_ID --execution EXECUTION_ID
opencoder-cli --no-auth project todos messages TODO_ID --execution EXECUTION_ID --seq 7 --offset 1024
```

`context` 默认读取全部关联，不依赖当前是否选择能力；返回 `record`（任务和全部评论）、`assignments`、`current_execution_id`、`executions`、`complete`、`issues`。每次执行包含原生 `detail`、`conclusion`、`events` 和 `full_fields`；Agent/Operator 还包含原 Session 与重组后的 `messages`，Team、TODO 工作流包含讨论轮次或工作项。`kind` 优先采用 Server 原生详情中的实际类型，不能从 ID 前缀或过期的关联提示猜测。长消息和省略字段自动翻页，数据较多时建议指定 `--execution` 并将 stdout 保存到文件。它只读取，不继续执行或修改看板。

节点离线或某项证据读取失败时保留已读内容，标记 `complete:false`、列出失败来源并返回 4；没有执行的 TODO 正常返回空证据。指定未关联的执行拒绝读取。`complete:true` 仅针对本次所选证据，不表示 TODO 已验收。`messages` 是原生单页分段数据，下一页的参数取自返回的 `next_cursor`；`--after-seq`、`--message-offset` 仍作为旧参数别名保留。
