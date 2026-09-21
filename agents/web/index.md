Commit: fc047704e4c583cb9e0c11293b3815916ce1659b

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配（`config_home`：Operator 执行 home，prompt/config 载入走 `Config::load_with_home`，drain 栈经 `DrainContext` 穿参）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- `src/api_agents.rs`、`src/api_agent_resources.rs` — agent 目录/卡片与资源文件 API
- `src/handle.rs`、`src/handle/drain.rs` — `SessionHandle` 与 drain 生命周期
- `src/auth_mw.rs`、`src/html.rs` — Bearer → Identity；SPA 产物内嵌与 `/static` 白名单
- `src/api_control.rs` — 节点对话 API
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`
- `spa/src/chat/` — 会话页（Operator/Agent 双模式 lane）
- `spa/src/fleet/`、`spa/src/schedule/` — 执行表与定时任务页
- `spa/src/agents/` — Agent 配置与资源页签
- `spa/src/dag/` — DAG 定义/运行页签与 React Flow 图：运行图 `dagProjection.js#graphFromSpec`（纯投影）与编辑器 `editor/canvasModel.js#specToCanvas` 的节点均声明固定盒（width + handles，**不声明 height**——声明会把内联高度烤进 wrapper，钳死 auto-height 卡片并错位 handle/fitView），边 id 用 `'e-' + src + '>' + dst`（`>` 不在 slug 字符集，杜绝连字符撞 key；`onConnect` 的 addEdge 路径同样显式传 id，勿依赖默认 getEdgeId）；RF 边是「两端节点 initialized（只需宽度）才渲染」的门控，勿再移除声明盒（jsdom RO shim 不回调，DOM 测试 `.react-flow__edge` 断言依赖声明盒）；编辑器 fitView 走 `useNodesInitialized()` 门控 + once-guard（仅挂载后首帧 fit，加步骤引起的重测量不再 refit）+ autoLayout `fitEpoch` effect，勿回退定时器
- `spa/src/dag/` spec 顶层 `max_concurrency` — 整跑并发上限（1..=30，缺省省略键、走服务端默认 4）：画布基础信息面板 `editor/stepInspector.jsx#SpecMetaForm` 可编辑；`editor/canvasModel.js#canvasToSpec` 透传 baseSpec 值（画布结构编辑不丢并发配置）；`specValidate.js` 镜像常量 `MAX_CONCURRENCY = 30` 在 `validateSpec` 校验（先于 name 检查）；只改定义、不影响在跑 run（run 持有 `dag_runs.spec_json` 快照，服务端整跑并发上限现状见 `crates/dag-runtime/src/runtime/scheduler.rs#schedule`）
- `spa/src/dag/dynamic/`、`spa/src/brain/workbench/` — 动态 DAG 与 Brain 工作台
- `spa/src/brain/workbench/layered/` — v4「分层能力画布」视图：`useRun.js` 用 `schemaVersionOf`/`isLayeredView` 判定版本（快照 `schema_version === 4`，或基础快照 404 时探测 `GET /api/brain/runs/:id/layered`，探测 404 回落 v3 路径），`run.jsx#BrainRunBody` 按版本分流 `LayeredRunBody`/`V3RunBody`/未知版本报错；v4 无运行事件流，未终止时按 3s 轮询 `/layered`（`layered` 标志由快照判定，v3 路径不轮询，终止/卸载即停），层决策明细懒加载 `.../layered/rounds/:layer`；`layered/model.js` 是纯投影（`layerNodeBox()` 声明盒、边不设 label，沿用 `spa/src/dag` 的 RF/jsdom 约束：两端 initialized 才渲染边、不启用 `onlyRenderVisibleElements`）
- `tests/` — 集成测试

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。

## 相关
- [control](../control/index.md)、[brain](../brain/index.md) — Brain 校验/快照发布/执行在后端
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
