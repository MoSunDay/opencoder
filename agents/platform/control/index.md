Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度；不依赖会话或 workload 运行时。

## 索引

- [routes.rs](../../../crates/control/src/routes.rs) — 路由装配
  （`build_app_with_auth` 公共鉴权与机器凭据分置）
- [bootstrap.rs](../../../crates/control/src/bootstrap.rs) — control.db / definitions.db
  / ontology.db 装配与就绪检查
- `src/role_gate.rs` — 角色门禁
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/transport/socket.rs`、`socket_io.rs` — 连接帧处理、心跳与背压写出
- `src/transport/report.rs`、`socket_report/` — 索引报告提交与逐批确认
- `src/transport/layered_tests.rs`、`layered_tests/` — 分层 wake 的 generation
  栅栏测试
- [api/session.rs](../../../crates/control/src/api/session.rs) — 会话执行面
  （`?kind=operator|agent` 泳道）
- `src/api/executions/` — 首轮提交、结果读取、能力要求与私有文件合同
  （submit / results / capabilities / private_context）
- [api/resource_admission/](../../../crates/control/src/api/resource_admission/mod.rs) —
  外部资源提供方准入、查询与释放
- `src/api/users.rs`、`api/users/tokens.rs` — 用户与独立 Token 管理
- `src/api/project/`、`api/project.rs`、`api/project_links/` — Project 中继与
  TODO 执行/能力引用
- `src/api/compat/` — 兼容路由（workflows / dag_instances / sessions …）
- `src/api/brain_runs/` — Brain 计划与运行；`v4/` 负责准入、派发、读取与事件确认
- `api/brain_runs/tui.rs` — TUI 能力目录投影
- `api/brain_runs/attachments/` — 通用图片附件上传与读取
- [api/stream.rs](../../../crates/control/src/api/stream.rs)、`api/streaming/` — SSE 与
  实例中继
- `src/api/catalog.rs` — 节点/定义目录
- `src/api/settings/` — Harness 与节点调度配置（Maintenance RPC 转发）
- `src/api/admission.rs` — `/ready` 准入与就绪快照
- `src/scheduler.rs`、`src/scheduler/`、`api/schedules/` — cron 调度与定义 CRUD；
  `api/scheduler_metrics.rs`、`seed_schedules.rs` 调度总览与遗留导入
- `src/catalog_guard/` — DAG / 二进制 / 能力 / Agent 目录写入与删除保护
- `src/ontology.rs`、`src/ontology_catalog/` — `/api/ontology` 挂载与
  Brain 目录→Ontology 同步
- `src/project_files/` — TODO 文件 HTTP 操作
- [release/resources.rs](../../../crates/control/src/release/resources.rs)、
  `src/resource_scope.rs` — 资源服务配置与只读 NFS 导出
- [release/outbox.rs](../../../crates/control/src/release/outbox.rs)、`outbox/` —
  派发外发箱、退避恢复与资源任务
- `src/release/ontology_schema/` — 共享 Ontology 结构启动核验
- `tests/e2e/` — 集成测试（`layered_api`、`schedule_api` 家族）

## 相关

- [agents/node](../node/index.md)、[agents/worker](../../execution/worker/index.md)
- [dag-binary](../../workflow/dag-binary/index.md)、
  [DAG 执行约定](../../../rules/04-dag-execution-contract.md)
- [agents/brain](../../orchestration/brain/index.md)、[运行协议](../../../crates/control/src/api/brain_runs)
