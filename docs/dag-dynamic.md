# Dynamic DAG Step

动态节点以一个 Agent 或 Wasm 模板执行一批输入。画布仍显示一个节点；所有实例成功后，按输入顺序把结构化结果聚合为数组交给下游。没有结构化结果的实例对应 `null`。

## 定义和派发

保存定义和派发沿用现有 DAG API。以下 Agent 节点读取派发请求 `input.items`：

```json
{
  "name": "batch-review",
  "steps": [{
    "name": "review",
    "kind": {
      "type": "dynamic",
      "source": {"type": "input", "pointer": "/items"},
      "template": {"type": "agent", "agent": "act", "prompt": "审查指定目标，返回 JSON。"}
    }
  }]
}
```

```json
{
  "node_id": "node-id",
  "input": {"items": ["审查 api 仓库，重点检查鉴权。", "审查 web 仓库，重点检查登录流程。"]}
}
```

Wasm 模板使用 `{"type":"wasm","command":"tool.wasm --format json"}`，输入使用 `[["--target","api"],["--title","hello world"]]`。每项直接追加到模板命令解析后的 argv，`hello world` 保持为一个参数。模块的发布、显式版本和沙箱配置沿用静态 Wasm 步骤。rootfs 自带运行器支持 OCI 的缓存配置参数；模块路径之后的 `--env`、`--dir` 等字符串都保留为模块参数。

上游决定输入时，使用 `{"type":"step_output","step":"discover","pointer":"/items"}`，同时把 `discover` 加入该节点的 `depends_on`。调度器等待它成功后，从其结构化输出读取 JSON Pointer。空指针表示整个输入或输出，`~0`、`~1` 分别表示 `~`、`/`。

模板仅允许 Agent 或 Wasm；超时放在逻辑节点的 `timeout_secs`，逐实例计算。单节点最多 1,000 项，全部校验通过后才展开。Agent 项必须为字符串，Wasm 项必须为不含 NUL 的字符串数组。缺失路径、错误类型、超限均使节点明确失败；空数组成功并输出 `[]`。

## 隔离、失败和恢复

每个实例以 `(step, index)` 标识，产物位于 `<run>/<step>/instances/<index>/`。展开前原子写入 `instances.json`，恢复始终复用清单中的输入和数量，跳过已成功的实例。

Agent 原始资源在执行前冻结。每个实例从冻结原文生成独立 `how.md`，依次追加模板 `how_append`、实例文本；恢复复用副本，不重复追加。Host 和 runc 都读取这个副本构建实际运行提示词，模板 `prompt` 继续作为公共执行提示。静态 DAG Agent 的 `how_append` 也只影响本次副本，成功后不再回写共享 Agent 资源或创建资源版本。

静态步骤和动态实例共享每个 run 的 4 个名额，按就绪逻辑节点轮询。一项失败后停止同组新派发、取消同组其他实例并等待退出；该逻辑节点失败、下游阻断，独立分支继续。失败引起的同组取消不会把 run 记成用户取消。用户取消则取消整个 run。

## 查询和控制台

| 接口 | 内容 |
| --- | --- |
| `GET /api/dag/runs/:id/steps/:step/instances?offset=0&limit=100` | 展开状态、总数、汇总计数和实例页；默认 100，最大 200 |
| `GET /api/dag/runs/:id/steps/:step/instances/:index` | 单实例输入、状态、会话、输出和错误 |
| `GET /api/dag/runs/:id/steps/:step/instances/:index/events` | 单实例 SSE，支持 `after` / `Last-Event-ID` 回放 |
| `GET /api/executions/:id/artifact?step=:step&index=:index&file=output.json` | 按实例读取产物，沿用原有鉴权 |

编辑器可选择动态来源及模板；派发弹窗为输入来源展示文本批次或 argv 批次。画布显示成功数/总数，未展开显示“等待派生”，空数组显示“0/0，无需执行”。抽屉提供分页实例 Select，只订阅当前实例，切换时清理旧订阅；恢复后按新开始时间清除旧执行状态和旧 Wasm 日志。

旧静态定义继续可读；不识别动态类型的旧 Worker 会拒绝定义，不会当作静态步骤执行。此功能复用现有事件存储，不增加数据库表或环境变量。

## 验证入口

- `cargo test -p opencoder-dag-runtime --test dynamic`
- `cargo test -p opencoder-worker --lib operations::query::tests::dag_step_events`
- `cargo test -p opencoder-control --test e2e dag_instances`
- `cargo test --test dag_e2e dynamic:: -- --nocapture`（包含真实 runc，需可用的 runc/rootfs 构建环境）
- SPA：`npm test -- src/dag`；真实浏览器：`node scripts/acceptance/dag_dynamic.js`（先构建 SPA、Server 和 Agent）
