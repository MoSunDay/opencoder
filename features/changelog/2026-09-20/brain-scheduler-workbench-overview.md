Commit: 2f6b202def6c5e75d1411143570784a1576b3407

# v3 大脑调度工作台按索引展示执行

## Context

v3 大脑运行保存的是轮次、能力 operation 和执行引用。工作台需要先让用户看见计划全貌与当前阶段，再沿 execution ID 查看节点侧的真实执行记录，避免把 DAG 步骤、Team 对话、TODO 项或产物正文复制到大脑状态。

## Change Summary

- Control 增加只读 `GET /api/brain/runs/:id/view` 投影，返回目标、输入名称、阶段、轮次、能力元数据和 operation 索引，不返回能力定义或子执行正文。
- v3 Brain 的通用 SSE 从 scheduler event projection 读取索引事件，并适配现有 `{seq, kind, data, ts}` 事件格式；事件仍只包含 ID、终态和摘要。
- SPA v3 工作台使用四节点摘要画布，当前轮默认展开、历史轮次折叠；所有 Agent、Team、DAG、TODO、Operator operation 均可点击 execution ID 打开复用的只读 `ExecutionView`。
- 托管执行面板隐藏独立重跑和执行控制，DAG 步骤日志、消息和其他明细继续通过通用执行索引从所属节点读取；v2 运行保留历史只读计划视图。

## Impact Surface

涉及 `control` Brain view/SSE、`worker` scheduler event query、SPA Brain workbench 与 TODO 托管视图。v3 的 pause/resume/cancel 仍作用于 Brain 统一调度，子执行不能从托管面板单独变更状态。

## Notes / Compatibility

v2 运行不会被迁移或改写，缺少 v3 view 投影时前端仍可使用 v3 snapshot 的 operation 索引。详细执行内容仍只通过 `GET /api/executions/{execution_id}` 及其既有子资源读取。

## Verification

- `cargo check --offline -p opencoder-control`
- `cargo clippy --offline -p opencoder-control -p opencoder-worker --lib -- -D warnings`
- `cargo test --offline -p opencoder-worker --test brain_scheduler_v3`（并行 workspace 资源竞争时存在既有终态中继超时；单测重跑通过）
- SPA Vitest：Brain v2 回归、v3 工作台 DOM、模型与重连测试通过（7 tests）；v3 专项测试通过（3 tests）。

## Related Docs

- [大脑调度工作台](../../brain/index.md)
- [web 模块](../../../agents/web/index.md)
- [worker 模块](../../../agents/worker/index.md)
