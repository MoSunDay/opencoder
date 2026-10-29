# 调度事件唤醒、退避窗对齐与轮询阶梯

继 2026-10-28 空闲降本后的第三轮稳态开销削减，rules/04 受理/调度间隔 ≤30s 预算不变（本次所有退避最坏 +5s）：

- **Worker 调度器事件唤醒 + 饱和退避**（`operations/queue/mod.rs`）：调度循环订阅 `loop_registry::subscribe()`，入队、槽位释放、准入/恢复变迁即时唤醒（`await_cycle`）；watch 语义保证 dispatch 期间发生的 notify 不丢失。节拍改为三级纯函数 `dispatch_delay`——上周期有实际启动 100ms，饱和（pending 无法启动）与空闲均退避 500ms。饱和节点从 10 次秒的全量恢复扫描（reconcile_capacity + brain/todo recovery + journal 快照）降到 2 次/秒，新工作可见延迟反而从最坏 500ms 降到即时。`dispatch_locked` 返回本周期启动数驱动分级。
- **Outbox 退避窗对齐下一次重试**（`release/outbox.rs` + `outbox/retry.rs`）：`Retries::next_due` 取最早到期重试；完整扫描结束且仅剩失败退避在等时，主循环 sleep `backoff_wait(next_due−now)`（100ms~5s 钳制），任何工作仍复位 100ms。30s 退避窗内约 300 次 DB 扫描降到 ≤6 次；outbox 是恢复通道（新提交由 API 内联派发），5s 封顶与空闲梯级语义一致。
- **SSE 结果流空页轮询阶梯**（`api/stream.rs`）：连续空页轮询间隔按纯函数 `poll_delay` 300ms→1s 封顶（取 1s 而非 2s，控制观看中空闲流的新事件可见延迟），收到任何帧即复位。空闲观看连接对节点的 RPC 从 3.3 次/秒降到 1 次/秒。
- **消除 config_cache 的 MUST-mirror 维护债**：core 导出 `config::domain_file_names()`（由路由表 `DOMAIN_FILES` 常函数派生，单一事实源）与 `config::candidates_with_home`；worker `state/config_cache.rs` 删除镜像表，指纹候选集的 config.json 链直取 core 派发、域文件直取 core 路由表，core 侧改动不再需要人工同步 worker 镜像，消除"指纹候选集与真实读取集漂移 → 配置热更新漏检"的隐性风险。

明确延后（本次不做，已有结论）：journal.summaries 累计增长（涉及索引上报与保留策略的产品决策，单独立项）、hub 单锁 + views() O(N) clone（千级节点前不构成瓶颈）、ontology 带外变更检测 30s→120s（commit 已记录为显式决策）。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 调度节拍三级分级（有启动/饱和/空闲） | `operations::queue::tests::dispatch_delay_tiers_started_saturated_and_idle` |
| notify 落在 dispatch 期间不丢失、下轮立即唤醒 | `operations::queue::tests::a_notify_landing_between_cycles_wakes_the_scheduler_immediately` |
| `next_due` 取最早到期重试、接受后清空 | `release::outbox::retry::tests::next_due_reports_the_earliest_pending_retry` |
| 退避窗 sleep 钳制（已到期 100ms 下限 / 5s 封顶 / 无退避 100ms） | `release::outbox::tests::backoff_wait_sleeps_until_the_earliest_due_retry_with_clamps` |
| SSE 空页阶梯 1s 封顶 | `api::stream::tests::poll_delay_ramps_on_empty_pages_and_caps_at_one_second` |
| 域文件名单与路由表一致（core 侧单一事实源派生） | `config::domain::tests::domain_file_names_match_the_routing_table` |
| 指纹候选集 = core config.json 候选链 ∪ 域文件 ∪ 额外 XDG 位置 | `state::config_cache::tests::candidate_files_cover_core_discovery_and_domain_routing` |

## 定向回归

仅运行本次改动及直接影响链路的定向检查，未运行全量测试。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder-worker --lib -- operations::queue` | 2 通过（2 项新增） |
| `cargo test -p opencoder-worker --lib -- state::config_cache` | 3 通过（1 项改写为 core 导出派生断言） |
| `cargo test -p opencoder-control --lib -- release::outbox` | 5 通过（含 2 项新增） |
| `cargo test -p opencoder-control --lib -- api::stream` | 1 通过（新增） |
| `cargo test -p opencoder-core --lib -- config` | 138 通过（含 1 项新增；导出未改变既有行为） |
| `cargo clippy -p opencoder-core -p opencoder-worker -p opencoder-control --all-targets` | 通过，零警告 |
| rules/04 门槛：`python3 scripts/ci/brain.py --output … scheduler`（真实 runc worker 调度链路 e2e） | 12 通过、0 失败、零 panic；派发链路节拍上限按构造 ≤500ms + 事件即时唤醒，受理间隔 ≤30s 预算保持 |
| 实机复测 | 见下方「实机复测」节 |

## 实机复测

release 构建，server(18081) + `--resources`(18082) + agent（`--max-runs 1`，黑洞 LLM 端点）：

- **空闲 60s 采样**（`/api/health` ok，节点注册在线）：server **0.067%**、resources **0.000%**、agent **0.067%**——与上一轮基线（0.067% / 0.00% / 0.05%）持平，空闲路径按设计不受本轮影响。
- **饱和场景**（本地 runtime 服务直连节点 RPC，`--max-runs 1`，1 个执行 running 挂在黑洞端点 + 2 个执行 pending，状态经 `inspect` 确认）：runtime agent 30s CPU **0.136%~0.167%**——无 10Hz 全量恢复扫描（reconcile_capacity + brain/todo recovery + journal 快照）的 CPU 特征；调度周期 10Hz→2Hz 的精确断言由 `dispatch_delay` 分级单测承担（在途执行自身的 10–100ms 内部轮询使外部 syscall 无法纯净切分调度器周期，如实记录）。
- SSE 阶梯（P3）无独立实机验收：需要 fleet 就绪节点 + 观看流，本轮 fleet 通道验收未完成；以纯函数单测 + 既有 SSE 传输链路为准。

环境备注：宿主机 python3.13 为 /usr/local 自编译动态链接布局（无 RUNPATH，libpython 解析到 /usr/local/lib，chroot 默认搜索路径不含），prepare 阶段需先把 libpython 补进 rootfs `/lib/x86_64-linux-gnu` 才能过 chroot 校验；属环境怪癖，与仓库代码无关。
