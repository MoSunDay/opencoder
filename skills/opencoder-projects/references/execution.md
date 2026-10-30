# 执行关联、运行编排与会话操作

本文档覆盖项目 TODO 的执行侧命令：关联已有执行、派发能力、Plan/Execute 运行编排，以及对关联会话的引导、读取与中断。示例适用于鉴权已关闭的 Server；`TODO_ID`、`EXECUTION_ID`、`CAPABILITY_ID`、`RUN_ID` 替换为实际 ID。命令参数细节用 `--help` 查询；连接与鉴权约定见 [SKILL.md](../SKILL.md) 与 [crud.md](crud.md)，退出码约定同 SKILL.md：4 表示请求拒绝（含 409）。

## 执行关联

关联记录只保存引用，不复制结论；以下命令维护 TODO 与原生执行之间的关联。

```sh
# 查看已有执行关联与当前执行
opencoder-cli --no-auth project todos links TODO_ID
# 关联已有执行；Operator 当前会话同样适用
opencoder-cli --no-auth project todos attach TODO_ID EXECUTION_ID
opencoder-cli --no-auth project todos attach TODO_ID EXECUTION_ID --capability-id CAPABILITY_ID
# 只移除引用，不取消也不删除原执行
opencoder-cli --no-auth project todos detach TODO_ID EXECUTION_ID
# 派发已登记能力并自动关联受理的执行
opencoder-cli --no-auth project todos dispatch TODO_ID --json @/tmp/dispatch.json
```

- `links` 返回 `assignments`（执行 ID、能力 ID、类型、名称、创建时间）和 `current_execution_id`。
- `attach` 关联已有执行，包括 Operator 的当前会话；`--capability-id` 可选。服务端核验实际执行与准入能力，重复关联幂等。
- `detach` 只移除引用，不取消也不删除原执行；删除 TODO 同样不取消其关联的原生执行。
- `dispatch` 派发已登记能力并自动关联受理的执行。正文形如 `/tmp/dispatch.json`：

  ```json
  {"execution_id":"稳定执行ID","capability_id":"CAPABILITY_ID","input":{},"node_id":null}
  ```

  执行 ID 由调用方预先确定并保持稳定；Fleet 回执固定能力与输入，重试复用原执行、不重复创建任务；同一执行 ID 换能力或换输入会被拒绝。已受理但关联失败时保留执行 ID，可重试关联。长输入写临时 JSON 文件用 `--json @/absolute/path.json`。
- 每次指派保留独立执行引用，新指派不覆盖旧指派。

## 运行编排（plan/execute）

计划运行与执行运行都由服务端在后台推进，接口先返回受理凭据，进度按异步回执确认。

```sh
# 发起一次计划运行；可选输入可携带 run_id 续跑既有运行
opencoder-cli --no-auth project todos plan TODO_ID
opencoder-cli --no-auth project todos plan TODO_ID --json '{"run_id":"RUN_ID"}'
# 在新建或续用的会话中驱动当前计划
opencoder-cli --no-auth project todos execute TODO_ID --json @/tmp/execute-input.json
# 运行历史与翻页
opencoder-cli --no-auth project todos runs TODO_ID
opencoder-cli --no-auth project todos runs TODO_ID --before-version 9
# 幂等取消一次运行
opencoder-cli --no-auth project todos cancel-run RUN_ID
```

- `plan` 发起一次计划运行，`execute` 在新建或续用的会话中驱动当前计划；两者返回 202 与 `run_id`（`prun-` 前缀，服务端生成），受理后仍需按异步回执确认进度。
- 可选 `--json` 输入对象可携带 `run_id` 续跑既有运行，不传输入直接开始新一轮；`/tmp/execute-input.json` 最简形式为 `{"run_id":"RUN_ID"}`。
- `runs` 读取运行历史，新版本在前、每页 20 条。`--before-version` 只返回版本号小于该值的运行，非法游标返回 400；翻页游标取返回的 `next_version`，`more` 表示还有下一页。
- `cancel-run` 幂等取消一次运行；对已结束或未知的运行返回 `{"cancelled":false}`，不视为错误。

## 会话操作（prompt/messages/events/result/interrupt）

以下命令先解析目标执行，再转发到原生执行接口，只作用于已关联的执行。

```sh
# 引导或继续会话；不带 --execution 时作用于当前能力最近一次执行
opencoder-cli --no-auth project todos prompt TODO_ID --json @/tmp/steer.json
opencoder-cli --no-auth project todos prompt TODO_ID --execution EXECUTION_ID --json '{"prompt":"补充要求","delivery":"queue"}'
# 读取会话消息，翻页参数取返回的 next_cursor
opencoder-cli --no-auth project todos messages TODO_ID --execution EXECUTION_ID
opencoder-cli --no-auth project todos messages TODO_ID --execution EXECUTION_ID --seq 7 --offset 1024
# 跟随执行事件，--after 为最后已读序号
opencoder-cli --no-auth project todos events TODO_ID --execution EXECUTION_ID --after 120
# 读取当前结果与请求中断
opencoder-cli --no-auth project todos result TODO_ID --execution EXECUTION_ID
opencoder-cli --no-auth project todos interrupt TODO_ID --execution EXECUTION_ID
```

- 目标选择：不带 `--execution` 时使用当前能力最近一次执行；`--execution` 必须是已关联执行，未关联返回 409、退出码 4；没有当前执行时先用 `dispatch`。
- `prompt` 继续或引导 Agent/Operator 会话，正文形如 `/tmp/steer.json`：

  ```json
  {"prompt":"必填非空","input_id":"稳定输入ID","delivery":"steer"}
  ```

  `delivery` 仅接受 `steer`（默认，插入当前轮）或 `queue`（排队下一轮）；省略 `input_id` 时 CLI 生成，重试保持同一 ID 避免重复投递。
- `prompt` 与 `interrupt` 仅适用于关联执行类型为 `agent` 或 `operator` 的会话；其他类型返回 409「不支持会话」、退出码 4。
- `messages` 按原生单页分段读取，翻页参数取返回 `next_cursor` 的 `seq/offset` 传给 `--seq/--offset`；旧参数 `--after-seq/--message-offset` 保留为别名。
- `events` 从 `--after` 序号之后跟随执行事件（默认 0），断线续传用最后已读序号。
- `result` 从所属执行节点读取当前结果；节点离线或读取失败返回实际错误。
- `interrupt` 请求中断当前会话，中断后可续跑，不是删除执行。

## 与上下文读取的分工

批量取证用 `project todos context`（见 crud.md 的「TODO 执行上下文」），本文件命令适合定向操作与轻量读取，轻量查看用 `links`、`result`、`messages`。两者都只读取或转发命令，不修改看板状态，执行结束也不自动完成 TODO。
