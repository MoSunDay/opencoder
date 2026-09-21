Commit: 7177f7987d3c3cdc7ed17864a4ffcf347bf4d182

# 分层能力画布（v4）的 CLI 读取面与根包进程级 e2e

CLI 把 v4 画布接进既有 brain 运行命令：`brain runs create` 只放行显式的 `schema_version` 3 或 4，其余（2、5、缺失、字符串 `"4"`）在计划期报错 `brain runs create requires an explicit schema_version: 3 or 4`，绝不静默回落；新增 `brain runs layered <id>` 与 `brain runs layered-round <id> <round>`，分别命中 `GET /api/brain/runs/:id/layered` 与 `.../layered/rounds/:round`。v3 的 `runs round`、`runs get` 路径逐字节不变，服务端 4xx 仍按既有退出码契约（4 = 服务端拒绝）透出。

根包新增进程级套件 `tests/brain_layered_e2e/`（真二进制 fleet + 共享 LLM 桩 + 真实 runc 激活），锁定三件事：准入面不回落、读取面与 CONTRACT §4 一致、真实画布的层屏障语义完整走通。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 裸 `/api/executions` 提 Brain 仍 409 且零模型调用；缺失/2/5 版本 409 且 `/layered` 404；非法画布（坏 id、未知能力、环、深度 4、深度 1 无父）400 且不建运行 | `raw_brain_submissions_stay_rejected_for_the_layered_canvas`、`unknown_schema_versions_never_fall_back_to_a_writer`、`invalid_canvases_are_rejected_before_any_dispatch` | `tests/brain_layered_e2e/admission.rs` |
| v4 视图键（`schema_version`/`layer`/`total_layers`/`layers`/能力按 id 去重/`operations`/`events`）、层明细 1/2 与越界 0/3 404、v3 运行走 `/layered` 404、v4 运行走 `/view` 409、命令白名单 400、CLI `layered` 与 `layered-round` 与 HTTP 一致 | `layered_view_and_rounds_read_a_real_projection` | `tests/brain_layered_e2e/surface.rs` |
| 真实 runc 画布跑到 `completed`：层屏障先等待后放行、逐层事件序、子执行 `brain_layered` 绑定与 `scheduler_output`、收口 summary、CLI 与模型调用计数 | `layered_canvas_holds_the_barrier_then_completes_through_the_closing_activation` | `tests/brain_layered_e2e/canvas.rs`（无 runc 时 SKIP 运行段） |
| CLI 计划映射：create 版本门禁、v4 读取路径、v3 round 路径不变 | `run_create_accepts_schema_versions_three_and_four`、`run_reads_map_to_the_locked_paths` | `crates/ctl/src/cmd/brain/ontology/tests.rs` |
| CLI 解析端到端：`brain runs create --json` 的 3\|4、`brain runs layered` / `layered-round` 路径与退出码 | `brain_run_create_accepts_v3_and_v4_and_layered_reads_match_the_contract` | `crates/ctl/tests/parse_project_brain_agents.rs` |

- `cargo test --test brain_layered_e2e`（`CARGO_TARGET_DIR=/data00/rust-build/cargo/layered-v4-private`，与本次 v4 真二进制同源）：5 passed / 0 failed。
- `cargo test -p opencoder-cli`：100 passed / 0 failed（13 个测试目标）。
- `cargo test --test brain_e2e`：2 passed / 0 failed，确认 v3 读路径未被 CLI 改动影响。
- 格式：`rustfmt --edition 2021 --check` 对 `crates/ctl` 本次改动与 `tests/brain_layered_e2e/` 全部文件无差异。
- Lint：`cargo clippy -p opencoder-cli --all-targets -- -D warnings` 与 `cargo clippy --test brain_layered_e2e -- -D warnings` 均零警告。
- 文档同步：`agents.md`（新增 `tests/brain_layered_e2e/` 条目）、`agents/ctl/index.md`（v4 读取子命令与版本门禁）、`features/brain/index.md`（CLI 读取面）、`docs/brain-orchestration.md`（新增 v4 层屏障契约小节）。
- 全量 `cargo test --workspace` 与发布回执按迭代收口另行记录，本条不据此宣告上线。
- 全轮收口回执：[brain-layered-v4.md](brain-layered-v4.md) — 合并全部测试清单，并附全量回归、clippy、构建、行数与 SPA 的实跑数字。
