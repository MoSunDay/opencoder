Commit: f2c9fa5（代码与文档一轮落盘；工作树与 daemon→server 拆分等并行迭代共享，其余路径随各自收口会话落盘）

# opencode-team：三路评审 P0 修复——resume 幻影追答轮 / 终态台账收敛 / 画像窄合并 / config 行数门

## Context

team 功能交付后三路只读评审（store+core / crates/team / web+SPA）认定 0 blocker、3 major + 1 工程红线，全部落在本功能的可靠性承诺上（resume 确定性、终态一致性、管理操作不丢失），暂缓合并。本轮按评审 P0 清单完成全部修复并复跑回归。

## 修复

- **M1 resume 幻影追答轮**（`crates/team/src/cursor.rs` + `runtime/stages.rs` + `runtime.rs`）：aligned summary 落盘后、turns 入账前崩溃的残态，旧游标会取 `Sub{k+1}` 按 aligned summary 的 ambiguities 重复派发成员。现推导规则改为**最大已存 summary 是权威**：`aligned` → 新增 `Stage::Record`（`stage_record` 读 plan+summary 补记 `meta.turns`，镜像 `stage_sub` 对齐分支，幂等）直接进 Closing，绝不派发幻影轮；未对齐才走原「最小无 summary sub-turn」前沿。
- **M2 终态台账不收敛**（`crates/team/src/runtime.rs`）：`terminal::finish` 先写 NFS 终态再翻台账，中间崩溃后重试走幂等早退永不翻台账，`(topic,node)` 行滞留 executing。早退分支现在先幂等调用 `finish_team_topic_run` 再返回，两写窗口必收敛。
- **M2' 评审复核追加：M2 修复在生产 HTTP 面不可达**（`crates/web/src/api_teams_topics.rs`）：`run_topic` 的早退补翻只在 spawn 之后生效，而其唯一生产入口的 resume 通道对终态非 error 话题在 spawn **之前** 409、cancel 落盘收束条件又只覆盖 executing/finished(error)——「disk finished(complete) + 台账 executing」残态此前无任何 web 路由可达，M2 沦为纵深防御。resume 的 409 拒绝分支现在先幂等 `finish_team_topic_run` 再拒绝：任意一次 resume 重试即收敛残态（该残态在生产 HTTP 面的唯一收敛入口）。
- **M3 画像全量快照回写丢更新**（`crates/team/src/profile.rs`）：旧实现开头 load 一次、数分钟访谈后整份旧快照 save 回去，窗口内的改队长/成员管理被静默回滚。改为**逐成员窄合并**：每次成功访谈后重读 team.json，只回写该成员 `capabilities`/`profiled_at`（+`updated_at`）立即落盘；成员被并发移除则丢弃该次结果；返回最终重读的团队。
- **R1 行数门红线**（`crates/core/src/config.rs` 806→696）：照 `config/agent.rs` 先例机械拆分——`config/provider.rs`（`ProviderConfig`/`HttpHeader`/`Endpoint`/`default_base_url`，48 行）与 `config/compaction.rs`（`CompactionConfig`/`OutputStreamlineConfig`+serde helpers，83 行），公共路径经 re-export 不变，零行为变更。

## 测试（规则 01/02 清单）

| 层 | 用例 |
|---|---|
| team 新增 3 | `runtime_flow.rs` `aligned_summary_crash_residue_records_turn_without_phantom_subturn`（手工构造 aligned+非空 ambiguity 崩溃残态；仅脚本 closing；断言 complete 收束、单 turn `{aligned:true, sub_turns:1}`、无 `1/1/` 目录、零成员派发）、`finished_topic_retry_converges_executing_ledger_rows`（人工回置 executing 后 `run_topic` 幂等早退补翻台账）；`termination_flow.rs` `profile_narrow_merge_preserves_concurrent_membership_edit`（自定义 TeamDispatcher 在首次访谈前并发加成员，断言三方成员/队长均不被回滚） |
| web 新增 1 | `api_teams.rs` `resume_rejection_converges_stale_executing_ledger`（手造 finished(complete) 话题 + 台账回置 executing 行；resume 409 后断言全行 finished，二次重试幂等收敛） |
| team 存量 | 26 用例全绿（dispatcher_flow 3 / layout_types 11 / runtime_flow 9 / termination_flow 3） |
| core | `cargo test -p opencoder-core` 全绿（含 `config/tests.rs` 经 re-export 编译不变）；clippy 0 warning |
| 回归 | `cargo test -p opencoder-core -p opencoder-store -p opencoder-team -p opencoder-web`：exit=0，110 suites 全 ok、0 失败；`cargo clippy`（同四 crate，--all-targets）0 warning；SPA `npm test` 328/328（一次高负载快照下 chat.dom/brainPanel 两个计时敏感用例失败，负载回落后复跑全绿，系并行 cargo 构建抢占 CPU 所致，本功能零 SPA 改动）；`scripts/check-spa-drift.sh` 无漂移 |

**M2' 修复后复跑**：`cargo test -p opencoder-web` exit=0（api_teams 10/10 含新用例）；`cargo test -p opencoder-core`（11 suites / 380 例）、`-p opencoder-team`（6 suites / 26 例）全绿；`-p opencoder-store` 30 suites 绿、唯 `schema_v4_migration`（2 例）失败——并行 project 批次在共享工作树把 `SCHEMA_VERSION` 17→18 的在途改动所致（该测试钉住 latest=17，随并行批次收口自愈；本批次零 store 改动，另见证到 project_store 一次在途抖动后自愈 8/8）；`cargo clippy -p opencoder-web --all-targets` 0 warning。

## 遗留（评审 P1/P2，未在本轮范围）

`has_editable_key` 补 3 个 team 键、`team_max_turns` 钳制 [1,999]、错误码映射细化、`read_topic_tree` sub_turn 数值排序、评审列出的 9 项测试盲区、SPA error 话题取消按钮、话题进度 SSE、executing 话题 server 重启自动收养等，见评审原文 TODO；`opencode-agents` 并行批次收口后补全量 workspace gate 闭合回归记录。
