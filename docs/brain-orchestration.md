# 大脑调度契约与运行协议

## 最小固定计划

`POST /api/brain/plan-defs` 保存版本。计划 ID 和版本构成固定引用，版本从 1 顺序递增，重复提交完全相同的版本幂等；同版本不同内容返回 409。

```json
{
  "id": "plan-review",
  "version": 1,
  "changelog": "并行收集两类证据后汇总",
  "author": "user",
  "created_at": 0,
  "tags": ["review"],
  "confidence": {"level": "unverified", "reason": "待验证", "evidence": []},
  "plan": {
    "schema_version": 1,
    "title": "并行审查",
    "objective": "生成有证据的审查报告",
    "inputs": {},
    "steps": [
      {
        "id": "requirements", "label": "需求审查", "purpose": "检查需求遗漏",
        "action": {"kind": "agent", "target": "act", "prompt": "检查需求完整性，返回审查证据"},
        "output": {"type": "string"}, "acceptance": "有明确结论和证据"
      },
      {
        "id": "implementation", "label": "实现审查", "purpose": "检查实现边界",
        "action": {"kind": "agent", "target": "act", "prompt": "检查实现与测试边界，返回审查证据"},
        "output": {"type": "string"}, "acceptance": "有明确结论和证据"
      },
      {
        "id": "report", "label": "汇总报告", "purpose": "形成交付物",
        "action": {"kind": "agent", "target": "act", "prompt": "根据两份输入汇总问题、影响和验证证据"},
        "inputs": {
          "requirements": {"schema": {"type": "string"}, "binding": {"source": "output", "step": "requirements"}},
          "implementation": {"schema": {"type": "string"}, "binding": {"source": "output", "step": "implementation"}}
        },
        "output": {"type": "string"}, "acceptance": "覆盖两份证据并给出交付结论"
      }
    ],
    "deliverables": {
      "report": {"description": "最终报告", "source": {"source": "output", "step": "report"}, "schema": {"type": "string"}}
    }
  }
}
```

`POST /api/brain/runs`：

```json
{"id":"brain-review-001","node_id":"node-example","mode":"fixed","objective":"完成本次需求审查","plan":{"id":"plan-review","version":1},"inputs":{}}
```

动态模式使用 `"mode":"dynamic"`，省略 `plan`，可传 `references:[{"id":"plan-review","version":1}]`。用户意图保留原始快照，相同 ID/意图重试不重复创建；更改意图应换 ID。

## 端口与动作

| 字段 | 约定 |
| --- | --- |
| `inputs.<name>` | `description`、`schema`、`required`；缺失必填值产生持久用户请求 |
| `steps[].inputs.<port>` | `schema`、`binding`、`required` |
| `binding` | `source: literal/input/output/item`；对应 `value/name/step`，可选 JSON pointer `path` |
| `output` 来源 | 隐含上游依赖，批量来源要求 `collect:true` |
| `when` | `{value: binding, equals: JSON值}`，未命中记 skipped |
| `foreach` | `{items: binding, key: JSON_pointer, allow_empty: true}`，字符串/整数项键不得重复 |
| `resources` | `[{key: "repo:example:main", mode: "read"或"write"}]`，不同运行共享同一命名空间 |
| `action` | `kind: agent/dag/todos/team`，`target`、`prompt`、定义快照、资源摘要、可选 `node_id` |
| `output_mode` | `text` 或 `json`；`output_pointer` 对原生结果作字段投影 |
| `max_attempts` | 默认 1，仅明确失败回执可重试；结果不确定时重发同一动作 ID |
| `deliverables` | 绑定实际输出、schema、可选精确 `expected`；不是只看全部步骤“运行过” |

原生输出：Agent 最后助手答案，Team `final_summary`，TODO 各项已验收 candidate，DAG 各步骤 `output.json`/`output.txt`。每类都有 `brain-result/output.json` 产物与 SHA-256；DAG 还包含各步骤元数据和原生输出。通过现有 `/api/executions/:id/artifact` 分块下载。

## 运行与恢复

- Fleet 协议 9 增加 `brain` 根执行及 durable outbox 消息；控制面执行索引仍仅五字段。旧决策树 API 保留。
- 根节点内部借用 TODO 工作流存储。状态、动作账本、来源游标、唤醒 revision 和因果事件在同一次提交中持久化。
- 每次有合法唤醒，根节点执行一次短激活；生产环境必须使用 runc，挂载同目录 `opencoder-cli`，以独立 context 文件运行 `brain activate-local`。固定计划调度不调用模型；动态规划只请求一次完整计划。
- Prepared 动作在派发前持久化。控制面按根串行处理派发/控制命令；受理未知时保留 ID。源节点在根提交成功前重放通知，提交后才确认 source 游标；分批轮转防止大量动作饿死后续通知。
- 根应用依赖变化后，立即准备新就绪步骤，不等待同批无关步骤。激活以 activation/control_epoch 防过期，不以所有事件共同递增的 revision 丢弃合法结果。
- 等待子执行、输入或资源时释放根节点运行槽。暂停阻止后续派发；取消进入 cancelling，等待所有在途执行有确定结束回执。
- 同节点重启恢复既有状态和动作；runc bundle 放在节点所属目录，恢复时清理所属残留容器。没有跨节点所有权迁移或运行中计划替换。
- 快照与事件水位在根锁内读取。实例页大小 100、历史事件页大小 100、版本页大小 20；V1 最多 200 个步骤模板、10,000 个展开实例、单次规划输出 1 MiB、结构化执行输出 256 KiB。

## 管理接口

- `/api/brain/library` 与 `/:id/stable`：能力集合和成熟度。
- `/api/brain/plan-defs`、`/validate`、`/:id/versions`、`/:id/versions/:version`、`/:id/stable`、`/:id/diff?from=&to=`。
- `/api/brain/runs/:id`、`/context`、`/actions`、`/instances?step=&offset=`、`/instances/:instance`、`/events-page?after=`、`/events`。
- `POST /api/brain/runs/:id/inputs`：`{name,value}`；已提供输入不可换值。
- `POST /api/brain/runs/:id/commands`：`{action:"pause"|"resume"|"cancel"}`。
- CLI 对应 `opencoder-cli brain library`、`brain plan-defs ...`、`brain runs ...`；各命令 `--help` 查看 JSON 入参选项。
