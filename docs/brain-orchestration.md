# 大脑里程碑计划与运行协议

修改调度实现时遵守 [规则 06：大脑调度约定](../rules/06-brain-scheduling-contract.md)；本页说明计划配置、请求和查询协议。

新计划和运行使用 `schema_version: 7`。`layers` 按顺序定义里程碑，每层必填名称、要做什么、目标和达成标准；`nodes` 是层内并行执行项，每个执行项通过 `layer_id` 归属一层，只选择一个 `capability_id`，名称与一句话任务说明取自能力库。首轮从第一层开始；达标后前进到紧邻下一层，必要时大脑可选择本层或任一已执行层整改。末层达标后可完成。

## 配置与能力

在画布配置大容器里程碑和并行执行节点。相邻层的线只表示正常前进顺序；回退目标由大脑根据执行证据与各层绑定能力选择。再通过表单填写计划名称、整体目标、工程输入与轮次预算。节点坐标只用于布局，不定义调度顺序。浏览器草稿保留画布与表单内容；保存创建不可变版本，运行须另行启动。

`POST /api/brain/plan-defs` 保存 `{id, version, plan, changelog, created_at, author, tags, confidence}`，同 ID/版本不可覆盖，相同提交幂等。`POST /api/brain/plan-defs/validate` 检查结构、能力及嵌套引用。

启动还会按候选执行节点的有效 `context_limit` 检查完整证据容量，未配置时为 128000 tokens。预算包含全部节点的结果上限及 16384 tokens 输出预留，同时受 1 MiB 决策正文和 2 MiB 传输帧限制；因此结构校验通过的多节点计划仍可能返回 413。此时须缩小计划或输入，或者选择确实支持更大上下文窗口的模型并配置其容量。已受理运行冻结该容量，不能靠改配置后恢复扩大预算；超限的人工输入也返回 413，且不写入运行事件。

```json
{
  "schema_version": 7,
  "title": "开发与验证",
  "objective": "交付经过验证的变更",
  "inputs": {"repo": "opencoder"},
  "max_rounds": 5,
  "layers": [
    {"layer_id":"coding","title":"Coding","task":"编码并审查变更","objective":"实现变更","success_criteria":"代码与证据达到交付标准"},
    {"layer_id":"testing","title":"测试","task":"运行测试并核对结果","objective":"验证变更","success_criteria":"测试与审查通过"}
  ],
  "nodes": [
    {"node_id":"code","layer_id":"coding","title":"编码","objective":"实现或整改变更","capability_id":"builtin-agent-act"},
    {"node_id":"review","layer_id":"coding","title":"并行审查","objective":"审查变更","capability_id":"builtin-agent-act"},
    {"node_id":"test","layer_id":"testing","title":"运行测试","objective":"返回测试证据","capability_id":"builtin-operator"}
  ]
}
```

能力 ID、输入输出及必填参数以 `GET /api/brain/library` 为准。运行准入冻结所引用能力的定义；每次派发必须覆盖目标层全部执行节点，每个节点调用其绑定能力一次。Agent、Team、DAG、TODO、Operator 和 Brain 复用已有执行接口。已保存的 schema 7 计划版本以 `plan-{plan_id}@{version}`、kind `brain` 出现在能力库；嵌套引用递归验证，根深度为 0，最多深度 3，子计划必须绑定真实父 operation。

## 决策、回退与轮次

所有同层执行进入终态后，大脑才评估当前层。`assessments` 必须只含当前 `layer_id`，说明达成标准是否满足及证据；首次派发的评估为空。前进或完成要求达标且执行成功。大脑结合完整计划、能力说明与执行结果决定下一层；回退或本层重试需要非空 `reflection`，并开启下一轮，令目标层及后续层的早先成果失效。历史执行和派发理由保留。新计划在保存与运行准入时自动补齐内部前进、回退路径；旧计划中的 `transitions` 保留供运行图校验，但决策条件不送给模型。没有节点自动重试或失败时自动取消同层任务。

大脑由运行创建、层屏障和人工输入唤醒；正在决策时收到人工输入，会废弃旧决策并重新唤醒。每次生产模型激活通过 OpenCoder agent loop 消费一份有限上下文并输出一个决策，落盘后停止，等待下一个事件。人工输入会记入运行事件和后续决策上下文，由大脑决定如何调度。在本层执行尚未全部终态时，大脑只能输出 `guide`，可通过可重放的事件向本层运行中的 Agent/Operator 发送即时引导；Team 的引导在下一次成员发问时生效。正在生成中的 Team 成员回答不会被打断。大脑同时记录后续调度调整，不能越过层屏障。DAG 图需要变更时，先停止原运行并提交新运行。

首轮为第 1 轮，正常前进不增加轮数。默认预算 5 轮，允许 1–32 轮；预算耗尽进入 `blocked`，不创建新执行。暂停或阻塞时可用 `set_round_budget` 增加预算，再 `resume`。每次派发增加 `activation`，operation/execution ID 包含激活身份，迟到和重复回执不推进当前决策。

输入绑定支持根输入、注册产物、终态 execution 的 JSON pointer，以及 `{kind:"value",value:...}` 的具体任务输入。普通及共享目录计划都只接受本次运行中该逻辑节点最新 activation 的终态引用，最新失败结果可以传给整改任务。能力只接收本节点任务与绑定输入；全局方法论、反思和转移决策留在大脑。模型上下文保留各层最近一次激活及有界摘要，完整历史留在日志中。非法决策最多自动纠正两次，仍不合法则阻塞；恢复时上次校验错误会反馈给模型。

限制：1–32 层、总共 1–256 个执行节点、每层 1–32 个节点、每节点一个能力。新计划不接受旧节点重试策略和旧 `edges` 连线。

## 能力输入、输出与文件交接

一次根 run 管理独立的 context 和执行索引，包含各层、返工和恢复；各能力默认使用自己的工作区。DAG 内部仍是单节点、单容器，同一 DAG 的全部步骤共享它自己的运行空间。

执行输入绑定可选 `source: "output" | "artifacts"`，省略时读取结构化输出。`path` 是 JSON Pointer；空字符串读取整体，`/` 读取名称为空的属性。文件引用同样通过 JSON Pointer 绑定：

```json
{
  "output": {"kind":"execution","execution_id":"dag-example","path":"/build/revision"},
  "files": {"kind":"execution","execution_id":"dag-example","source":"artifacts","path":""}
}
```

子执行明细保存 `scheduler_output`、`scheduler_artifacts`。决策 context 的 `execution_artifacts` 按 execution ID 保存文件引用，每项包含 `execution`、`step`、可选 `index`、`file`、`bytes` 和 `sha256`。DAG 的索引包含声明的业务文件、步骤输出及元数据，也包含动态实例文件。其他叶子能力只纳入自己工作区声明的文件；没有文件时返回空数组。

文件通过 `GET /api/executions/{execution.id}/artifact?step={step}&file={file}` 读取；动态实例另传 `index`。消费者核对引用的执行身份、字节数和 SHA-256，文件仍保存在其所属节点。路径逃逸、软链接、文件缺失和声明不符都返回实际错误。

叶子能力的结构化输出与文件引用合计最多 16 KiB。较大的索引与嵌套 Brain 使用 `step: "brain-result"` 的清单引用，下载后读取 `{"schema_version":1,"execution":...,"artifacts":[...]}`，按其中的引用继续取文件。清单文件名包含自身摘要，恢复不覆盖旧清单；它只压缩索引，不裁剪成功输出，证据仍超限时执行失败。

`brain-result` 属于运行时产物接口的保留步骤名称，新 DAG 不能将它作为业务步骤名，避免与清单下载冲突。

DAG 的静态二进制、Agent 与动态实例都取得完整的冻结命名输入。只读 `OPENCODER_STEP_CONTEXT` 包含 `run_id`、`inputs` 和声明依赖的 `steps`；动态实例另有 `instance: {index,input}`。恢复使用原输入和身份，已保存输入漂移时拒绝执行。

新 DAG 和采用 `execution_io_version: 1` 的根 Brain、托管子执行要求节点声明 `execution_io_v1`。根协议在受理时固定，子执行与嵌套计划继承；缺少标记的历史根继续发送原 context，不能使用 `source: artifacts`。新 Worker 收到旧协议上下文时也不会生成新绑定。

先发布 IO reader 过渡版本，再启用 writer；后续回滚使用支持新绑定的过渡版本。发布包及七个二进制的 `execution_io.reader/writer` 必须一致，发布工具在启动候选前检查回滚基线，并拒绝激活无法读取保留 Runtime 协议的旧 Server。旧任务仍按原 Runtime 归属执行，重试复用原 Assignment 的完整请求，不改写历史数据或派发指纹。

## 查询与历史

- `POST /api/brain/runs`：`{schema_version:7,id,plan:{id,version},inputs,node_id}`，也支持内联计划。顶层 `node_id` 是可选的物理执行节点约束，显式指定时普通子操作沿用冻结请求中的该值；省略时普通子操作各自选点，不绑定调度器选中的根节点。计划和 operation 中的 `node_id` 仅标识逻辑步骤。嵌套 Brain 仍跟随父 Brain 所属节点。相同 ID/意图重放回执，冲突返回 409。
- `GET /api/brain/runs/:id/layered`：冻结计划、层级、运行、操作索引、能力元数据和事件。
- `GET /api/brain/runs/:id/layered/rounds/:layer?activation=:id`：指定层激活的 `visit`、历史 `visits`、层里程碑评估及各节点执行索引；省略 activation 返回最新一次。路径中的 `rounds` 是历史接口名称，参数仍为层号。
- `GET /api/brain/runs/:id/events-page` 与事件流：运行日志。
- `POST /api/brain/runs/:id/commands`：暂停、恢复、取消，或 `set_round_budget`。
- `POST /api/brain/runs/:id/inputs`：`{text}`，记录人工输入并唤醒大脑；不直接向子执行发送命令。

运行主视图展示画布与当前轮次、层级状态。右侧详情抽屉按轮次展开每条执行记录，包含理由与输入绑定；点击执行 ID 复用 `ExecutionView` 查询明细，人工输入也在该抽屉内提交。历史 schema 4/5/6 计划与已结束运行只读；编辑旧计划显式创建 schema 7 新版本，不改写旧数据。升级和回滚前检查未结束运行，不支持的运行须在原 Runtime 收敛；历史运行仍由所属 Runtime 提供查询。

## Operator 共享工作区

计划可设置 `shared_workspace: true` 和 `adjacent_only: true`。前者只接受 Operator，固定到根 Brain 节点并共享一次运行的目录；后者禁止返工跨越上一层。Operator HOME 仍独立。运行目录和各 activation 目录由 Worker 受理时固定，通过 `OPENCODER_RUN_CONTEXT` 读取 root、step、run_id、execution_id、activation 及绑定 inputs。输入和计划在共享根目录冻结，旧 activation 证据保留。

环境恢复时先让根运行保持 paused/blocked，再 resume 原失败 Operator；Worker 接收同一执行更高序号的新终态，保留目录和轮次。待结果恢复后显式 resume 根运行。改变任务或修复方案则属于返工，需要新 activation。共享目录归属为节点自有字段，普通 annotate 不能修改或删除。

下载证据使用 `GET /api/executions/{id}/artifact?step=workspace&file=artifacts.json`，再按清单逐文件下载；Brain ID 指向共享根，Operator ID 指向该 activation。额外文件须声明 bytes 和 sha256，接口核验内容并拒绝软链接及目录逃逸。此模式要求节点声明 `brain_workspace_v1`。
