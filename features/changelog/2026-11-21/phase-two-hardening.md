# 阶段二：控制面与传输层加固五项收口

一句话：P2 闩锁读收敛 + P1 四项（host HTTP 客户端背压超时、sse_dag Lagged
契约统一、预留孤儿对账 GC 与辅助 server 优雅关停、周期 PASSIVE WAL
checkpoint）一次闭环，代码、测试、规则评审与发布换装同轮完成。

## 变更清单（功能 → 测试映射，规则 01/02）

| 提交 | 内容 | 定向测试 |
| --- | --- | --- |
| `2f32a21df` | P2：worker 持久化闩锁读收敛——`snapshot()` 与受理检查不再直读 `inner.persistence_error`/deps 内部状态，统一走 `runtime::persistence` 的 `node_error`/`error` 访问器（own→deps 合并次序收归模块所有，行为不变） | `opencoder-worker --lib runtime::persistence_tests`（新增 `accessors_merge_own_then_deps_latch_into_snapshots`：own 优先合并、deps 回落、snapshot `resource_error` wire 文本断言） |
| `c359435c1` | P1-a：agent host HTTP 客户端 builder 加 300s 总超时背压——此前无 builder 级超时，任何未来缺失 per-request 超时的调用点可无限挂起并占用 fleet channel 许可；现有调用点（inventory/runtime 15s per-request、admission 10–45s 包裹）经 `RequestBuilder::timeout` 覆盖语义不受影响 | `opencoder-agent --bin opencoder-agent host::tests::http_backstop`（stalling server：builder 级超时以 timeout 错误浮出、per-request 300ms 覆盖 300s 背压、常量宽于全部调用点预算） |
| `f19b8f811` | P1-c：`sse_dag` 直播流 `Err(Lagged)` 不再被 `.ok()` 静默吞掉——合成带 `lag` 数值标记的 `error` 帧（payload 与 `api::map_broadcast_result` 共享 `lag_payload` 单源），帧 id 复用回放头作为重连游标，绕过 exact-seq 去重直达客户端 | `opencoder-web --test dag_api_sse`（新增 `sse_live_lag_is_surfaced_as_error_frame_with_lag_marker`：先开流后 400 事件洪峰 > ring 256，断言 lag 帧 `event:error` + `payload.lag>=1`）+ `--test broadcast_lag_handling`（既有 3 例回归） |
| `f5ca531d6` | P1-b：hub 预留孤儿对账 GC（60s 周期比对持久 Pending 集，整页 ≥10000 跳过、受理宽限窗 150s 防 reserve-先于-持久化竞态误删）+ 辅助资源 server（resources/todo_resources）优雅关停（复用主 server `shutdown_signal`）+ 删除无调用者的 `clear_reservations` 死代码 | `opencoder-control --lib transport`（新增 `reconcile_reservations_releases_only_non_pending_entries`：宽限窗内不释放、过期释放、幂等、`pending_ids` 过滤）+ `--test aux_graceful_shutdown`（in-flight 慢请求在 shutdown 触发后仍完整送达、`serve_until` 干净返回）+ `--test resource_service_native` 回归 |
| `d0e87cbca` | P1-d：周期 PASSIVE WAL checkpoint——`Database::open` 按路径挂载（60s/1MiB 门限、独立可写诊断连接 2s busy timeout、busy 即跳过、按路径 OnceLock 防重）；PASSIVE 只回填已提交帧（文件高水位保留、写者复用），TRUNCATE 仍 CLI-only（writers_stopped 门槛不动） | `opencoder-database --test checkpoint`（阈值下跳过、超阈值合并 `checkpointed_pages>0 && after<=before`、写锁持有时不阻塞不报错、`wal_path` 布局）+ 全 crate 22/22 + `opencoder-store --lib` 75/75 |

测试范围声明：以上均为本次改动及其直接影响链路的定向回归（规则 02）；
`opencoder-store --lib` 因消费 `Database::open` 池路径纳入；未跑全量。

## 规则 06 评审记录（P1-b 预留对账 GC，用户已委托自行评审）

- **不变量「预留数量参与选点，防止并发受理重复占用名额」未弱化，反而加强**：
  对账只释放与持久 Pending 集脱钩的孤儿预留，使 `reserved_loops` 回到与
  持久状态一致的真值；此前孤儿预留永久虚高、系统性低估节点可用容量。
- **竞态防护**：`submit.rs` 受理序为 reserve → 持久化（有意设计，防持久化
  等待窗内并发提交重复占容）。预留记录携带 `at_ms`，对账仅释放宽限 150s
  （> 两轮 60s 周期）以上的孤儿，杜绝误删在途受理的预留。
- **整页保护**：持久 Pending 查询页满（≥10000）时跳过本轮——集不完整时
  对账可能误删活预留，宁可不跑。
- **开机重预留恢复语义未动**（bootstrap 对全部持久 Pending 重 reserve）；
  `acknowledge`/`release_claim`/unregister 三条既有释放路径未改。

## 规则 04 评审记录（P1-d 周期 checkpoint，用户已委托自行评审）

1. **不触及 DAG 协议、调度、资源分发与工作区**：checkpoint 为存储引擎
   层维护操作，不改任何执行路径、派发归属或工作区内容。
2. **不阻塞执行写路径**：独立诊断连接 + 2s busy timeout，争用即跳过；
   `wal_autocheckpoint` 兜底，写者永不等待 checkpoint。
3. **恢复语义不变**：PASSIVE 合并已提交帧属引擎级操作，journal/WAL 恢复
   语义、快照核对与拒绝规则不受影响。
4. **TRUNCATE 保持 CLI-only**（`ctl` maintenance 门槛 `writers_stopped`
   不动），部署换装的严格截断路径未变。
5. **验收**：定向回归即上表；无协议变更故不触发 runc e2e 重验；发布后
   soak 观察期 WAL 曲线纳入常规核验。

## 已知边界

- PASSIVE checkpoint 不截断 WAL 文件（turso/SQLite 语义，仅 TRUNCATE 截
  断）；高水位由写模式决定，本轮目标是已提交帧及时合并、帧复用可控，
  实测 `checkpointed_pages=326、after<=before`。
- 辅助 server（resources/todo-resources）优雅关停覆盖 HTTP 面；NFS 导出
  子进程生命周期独立，维持既有行为。

## 状态与后续

- 代码与测试：五提交全部通过定向回归 + Clippy 零新告警。
- Ontology（规则 09）：五项均为内部行为修复，无公共能力/CLI/API 入口/
  依赖变更，无需同步。

## 发布上线记录（2026-11-21 14:54–14:58）

- 构建 `d0e87cbca`：server sha256 前 16 `4cc6a457bb4be4bd`、agent
  `30695913c3159df9`（codegen-units=1 + thin LTO 全量 release；本机
  120s 工具窗口限制下，大依赖 turso_core 以单包攻坚方式在 89s 窗口内
  完成，其余增量续建）。
- 回滚点：`/usr/local/bin/opencoder-server.bak.20261121145438`（阶段一
  点 `.bak.20261121103223/25` 仍在）。
- 滚动顺序：server 先滚（14:54:38 换装 → restart → active、GET / 200、
  运行指纹 `4cc6a457`），agents 后滚（node02/node04 restart → active、
  运行指纹 `30695913`）。
- 观察期 3 采样（14:55:20 / 14:56:46 / 14:58:07）：GET / 200 ×3；双节点
  online/ready/idle、`reserved_loops=0`（预留计数口径正常）；换装后
  ERROR = 0——仅 agent 重启时刻 2 条 `node channel failed`（Connection
  reset）为预期（与 soak churn 段结论一致：节点重启一一对应）。
- 周期 checkpoint 生效口径：当前 server 数据目录 WAL（control.db
  32,992 B、definitions.db 267,832 B）均低于 1 MiB 阈值，周期任务按设计
  处于 Skipped 静默；压力期水位越限后将出现 `periodic wal checkpoint
  merged` 日志（测试已覆盖该路径）。
