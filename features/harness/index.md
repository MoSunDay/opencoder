Commit: 2491657d33c384dddcabf4d12ab4cd8822ccaf81


# Agent Harness

Agent 可以使用 OpenCoder 原生执行器，或将需求交给执行节点上的 Codex 二进制。两者都写入 OpenCoder 的会话、消息和事件；Web 复用现有 Steps、Thinking、Function call、Say 折叠展示。

## 使用

```sh
opencoder --wrap codex --cmd "检查当前项目并完成需求" \
  --envs MODE=review --envs 'NOTE=包含空格和=的值'
```

`--cmd` 是一次性需求，CLI 输出文本后退出。`--envs KEY=VALUE` 可以重复，按第一个等号拆分，重复的键以后者为准；值可以为空。需求经 stdin 传给 Codex，不作为 shell 命令解释。`--wrap opencoder` 显式使用原生执行器；省略时跟随 Agent 的默认 Harness。

Server Web 的「Agent 配置」顶部按 Agent 列表、Agent Harness、Harness 管理、Runner 管理、NFS 配置切换。列表逐个展示内置和自定义 Agent 引用的 prompt、skills、tools、memory 的名称、当前版本、NFS 相对路径和内容；Agent Harness 页管理默认执行器及命名配置绑定。启动弹窗和「全部执行」仍可覆盖本次 Harness 选择。

Harness 管理分别保存默认及命名 Codex 配置，包括二进制路径、模型、推理强度、授权槽位、sandbox、approval policy 和环境变量，并显示配置修订号。留空项使用节点 Codex 自身的配置。配置在任务接受时形成私有快照，排队、续聊和分叉保持原值，修改只影响后续新任务。受管 Codex 不接受单次启动的 env 或原生模型覆盖；原生 Agent 的独立环境变量配置仍可用。管理值保存在 Server 私有定义库，随分配下发到 Node，不写入 Agent NFS 引用卡。

## 执行与恢复

- 节点优先使用受管配置的二进制路径，否则从其 PATH（或配置环境变量覆盖的 PATH）定位 `codex`，执行 `codex exec --json`。Codex 使用节点已有的认证、配置、工具和权限策略，无需 OpenCoder 原生模型凭据。
- PATH 中的入口必须在当前进程输出 exec JSONL。启动 screen 或独立终端后退出的包装脚本需要改为实际 Codex 二进制入口；可用本次 `--envs PATH=...` 指定含该入口的目录。
- 正常续聊使用 `codex exec resume <thread-id> --json`；分叉使用 `codex exec fork <thread-id> --json`。thread ID、输入提交位置和执行状态写入私有会话状态。
- queue 顺序执行；steer 终止当前进程树后，向原 thread 提交新指令。取消关闭尚未完成的工具条目。Linux CLI 和 Node 复用进程监管，回收脱离原进程组的后代进程。
- 意外退出后不自动重复提交已发送的需求。有 thread ID 时可以显式提交新指令继续；未知提交状态且缺少 thread ID 时明确报错。进行中的会话不能分叉。
- Codex thread 的 Agent 指令固定。切换其他 Agent 需要新会话；清空上下文会建立新 Codex thread，保留既有 Plan → Act 交接规则。
- 缺少二进制、非零退出、损坏或不支持的 JSONL、缺失终止帧、流超时都会报告错误。

## Agent 资源

执行前，将引用卡选择的 prompt、skills、tools、memory 版本复制到工作区 `.opencoder/runtime/<session>/<agent>/<snapshot>/`。soul/how/output 及 memory 合成指令；同时提供真实文件路径，保留 skill 目录、相对链接布局和工具可执行权限。额外开放的工具池遵循 `tools_scope`。

OpenCoder 和 Codex 共用资源准备入口。会话恢复优先读取固定快照。Codex 的资源更新影响后续新会话；原生 Agent 保留显式配置热重载时刷新工具池的行为，新快照不会改写已分叉会话的资源。运行目录加入 Git 的本地 `info/exclude`。环境变量保存在受认证保护的 Harness 管理配置与节点私有执行状态中，公开执行详情不返回其值。

独立 Agent、原生 subagent、Team、DAG Agent step、TODO 和 Project 的 Agent 执行都经过共享 Session 入口，使用各自 Agent 的 Harness 配置。

## Project 与交付文件

Plan 使用 plan Agent 的 Harness，Execute 使用所选执行器；资源引用仍需有效，但纯 Codex 运行无需原生模型凭据。同一 Agent、Harness 和资源版本的连续 Execute 恢复同一会话，切换后创建新会话。

Codex Execute 收到本次交付清单路径，将工作目录相对路径写为 JSON 数组即可登记文件。清单最多 64 KiB、100 项，文件必须留在工作目录内。系统在本轮结束时保存独立副本、大小与 SHA-256，工作副本后续修改不会改变历史交付。Plan 只生成方案，不绑定交付清单路径。记录中可查看 Harness、Codex thread 和模型来源；Codex 内部请求不作为原生模型调用记录展示。

## 升级与回滚

Fleet 协议为 v7；Server 和 Node 必须成套升级。跨版本连接直接拒绝，避免旧 Node 忽略受管 Codex 参数和排队契约；Server 默认日志显示拒绝原因。Codex 入口和认证必须在实际执行节点配置，并保持使用前台 exec JSONL 入口。

Codex wrap 的可空运行态列使用 schema 22；命名 Harness、注册 Runner 和节点队列复用现有存储，不另增业务表。历史会话默认保持原生。切换前保存停写后的数据备份；回滚时同步恢复整套二进制，并按需要使用升级前数据副本。旧程序可读取历史原生会话，但不能用于继续新的 Codex 会话。发布/回滚工具见 [平台部署](../../docs/agent-platform.md)。

## 消息协议

Codex 的 reasoning、agent_message、command_execution、file_change、mcp_tool_call、collab_tool_call、web_search、todo_list 和 error 分别转换为 OpenCoder 推理、文本、工具和状态消息。工具 ID 带独立 turn 前缀，避免 Codex 续聊重用 item ID 导致串联。累计文本只生成新增 delta；完整消息与用量持久化后，页面刷新仍使用相同折叠结构。

协议依据本机 Codex 0.153.2 的 exec JSONL 实现验证。可复用浏览器验收脚本：

```sh
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/harness/codex.js
PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/harness/codex.js /absolute/path/to/codex
```

脚本启动临时 Server 和 Node，经真实浏览器验证顶部 tabs、NFS 资源内容和版本、Agent Harness 切换、受管参数保存、非法环境零写入、节点并发与队列策略，以及工具折叠（含失败后恢复）、刷新、续聊与 390px 布局。真实 Codex 模式还验证 Project Plan → Execute、清单登记与不可变交付副本；不提供二进制路径时使用确定性进程夹具。

相关逻辑：[session](../../agents/session/index.md)、[CLI](../../agents/local/index.md)、[Web](../../agents/web/index.md)、[worker](../../agents/worker/index.md)。

## 业务 Runner

已有业务工具可登记前台入口，通过 DAG Runner 执行。模型配置、资源版本和安装文件在受理时固定；阶段和 Codex 消息进入同一折叠回放，报告支持下载。执行成功与业务准出结论分开显示。已开始且无有效完成收据的任务需要显式业务重试，避免再次提交模型。完整约定见 [注册 Runner](../../docs/registered-runners.md)。
