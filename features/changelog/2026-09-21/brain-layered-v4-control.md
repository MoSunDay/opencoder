Commit: 7177f7987d3c3cdc7ed17864a4ffcf347bf4d182

# 大脑分层能力画布（v4）的控制面准入、读取与投递

控制面为 `schema_version: 4` 的分层能力画布补齐 v3 同构的接缝：准入时按 `schema_version` 分支（`==3` 走 v3、`==4` 走 v4、其余显式 409 `migration required`，绝不回落到 v3/v2），v4 只能落在广告了 `brain_scheduler_v4` 的节点上，读取与生命周期命令全部转发给持有该运行投影的节点。控制面仍然不持层划分、不持 generation：只归一化能力目录、创建普通子执行、搬运回执，层屏障与栅栏留在节点。v3 的路由、回执、读路径与错误语义逐字节不变。

## 变更摘要

- **准入与路由**：`runs::create` 按 `schema_version` 分派；新增 `require_brain`（3|4 才放行通用执行命令）替换 v3 专用门禁，`v4::create` 复用 `claim_request("brain-run", id, fingerprint)` 做幂等：同一 intent 重放回执，异 intent 409。
- **锁定 HTTP 面**：新增 `GET /api/brain/runs/:id/layered`（视图）与 `GET /api/brain/runs/:id/layered/rounds/:round`（层明细），键与 CONTRACT §4 一致；两者只在运行为 v4 时成立，v3/未知运行 404，v3 的 `/view`、`/rounds/:round` 遇 v4 运行 409 `migration required`，两个版本互不跨服。
- **能力冻结**：`executions::capabilities::required` 增加 `BRAIN_V4`（v4 根运行、以及带 `brain_layered` 绑定的嵌套子执行都要节点广告协议）；探针不广告时按既有选点语义失败（503，最终错误点名缺失能力），不把 v4 派给 v3-only 节点。
- **目录与校验**：`api/catalog.rs`（brain 定义解析）与 `executions::capabilities::required` 对 `schema_version == 4` 走 `opencoder_brain::layered::validate_request`，`plans::save` 接受 v4 计划（层、单层宽度、深度、能力唯一可用都在准入时判定）。
- **投递与回执**：`api/brain_runs/effects.rs` 把 `layered_` 前缀的动作路由到新模块 `api/brain_runs/v4/delivery.rs`；控制面处理 `layered_wake`/`layered_dispatch`/`layered_cancel`/`layered_terminal`，回写 ack 与副作用 `layered_wake_ack`/`layered_authorize`/`layered_receipt`/`layered_dispatch_ack`/`layered_cancel_ack`，激活经 `layered_context`（root↔child 读走 `layered_summary`/`layered_output`）。
- **生成栅栏**：一次 `layered_wake` 只确认本次激活实际准入的 generation；投递期间冒出更新 Ready 轮次时，旧 wake 不得吞掉新轮的确认（`transport/layered_tests.rs`）。
- **读取**：`v4/read.rs` 用与 v3 同名的 `snapshot`/`events` 动作读节点投影（节点按运行自身的 `schema_version` 选择负载），事件页带上限翻页；`/events`（SSE）沿用共享执行事件流，节点对 v4 运行按其事件日志分页。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 视图/层明细读节点投影：键、层划分重算、越界层 404 | `layered_view_and_rounds_read_the_node_projection` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 不跨版本服务：v3/未知运行读 layered 404，v4 运行读 v3 呈现 409 | `layered_routes_never_cross_serve_another_schema` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 事件流与 `?offset=` 快照都委托节点投影 | `layered_events_and_snapshot_routes_delegate_for_v4_runs` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 准入要求 v4 广告（v3-only 节点 503 并点名能力）、冻结能力 scope、同 intent 重放回执 | `layered_admission_requires_the_v4_advertisement_and_freezes_the_scope` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 未知/历史 `schema_version` 显式报错，不建索引、不触发模型 | `unknown_or_legacy_schema_versions_are_explicit_errors` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 请求成形：环与未知能力在准入即 400，不进入选点 | `layered_create_requires_a_well_formed_request` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 嵌套准入：深度 ≤ 3 且必有 parent，合法嵌套冻结父绑定 | `layered_nesting_is_bounded_and_requires_its_parent` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 生命周期命令只放行 pause/resume/cancel，其他 400；未知/历史运行 409 | `layered_commands_forward_only_the_three_lifecycle_actions` | `crates/control/tests/e2e/layered_api/commands.rs` |
| wake 只确认自己激活的 generation；迟到 wake 不吃掉新一轮 | `layered_wake_acknowledges_the_generation_its_activation_admitted`、`stale_layered_wake_does_not_acknowledge_a_new_ready_generation` | `crates/control/src/transport/layered_tests.rs` |
| 能力探针：v4 运行与 `brain_layered` 子执行都要 `brain_scheduler_v4`，旧探针不冒充 | `layered_runs_require_the_v4_advertisement`、`old_positive_probes_do_not_advertise_new_protocol_operations` | `crates/control/src/api/executions/capabilities.rs` |
| v3 回归 | control 全量 lib 与 e2e 套件（含既有 brain/dag/executions 家族） | `crates/control/src/`、`crates/control/tests/e2e/` |

- `cargo check -p opencoder-control --all-targets`：通过。
- `cargo test -p opencoder-control --lib`：74 passed / 0 failed。
- `cargo test -p opencoder-control --test e2e`：204 passed / 0 failed（其中 v4 `layered_api` 8 项）。
- `cargo clippy -p opencoder-control --all-targets -- -D warnings`：零警告。
- 格式：`rustfmt --edition 2021 --check` 对 `crates/control/**` 本次改动与新增文件无差异。
- 契约 §5 交叉门禁：`cargo check -p opencoder-worker` 通过；`cargo test -p opencoder-brain --test layered`（11 passed）、`cargo test -p opencoder-store --test brain_layered_v4`（5 passed）；`bash scripts/check-spa-drift.sh` 输出 `spa dist: no drift`。
- 全量 `cargo test --workspace` 与发布回执按迭代收口另行记录，本条不据此宣告上线。

## 兼容与边界

- v3 未改语义：新增分支全部是 `== 4` 判定或新模块/新路由；`require_brain` 对 v3 与历史运行的返回值与旧 `require_v3` 一致。
- Store `SCHEMA_VERSION` 未由本轮推动（合并线上为 28，来自同批合并的发布提交自身），控制面不写层划分，层一律重算。
- 控制面不链接 worker；`/events` 仍复用共享 SSE 通道，v4 运行的事件负载由节点按运行版本给出。

## 相关文档

- [features/brain/index.md](../../brain/index.md) — 分层能力画布的用户可见行为
- [agents/control/index.md](../../../agents/control/index.md) — 控制面模块索引
- [features/changelog/2026-09-21/brain-layered-v4-worker.md](brain-layered-v4-worker.md) — 节点侧与持久化回执
- [features/changelog/2026-09-21/brain-layered-v4.md](brain-layered-v4.md) — 全轮收口：合并测试清单与全量回归/clippy/构建/SPA 实跑数字
