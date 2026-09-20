Commit: 6956b99332ac7ddc55394a9d55c47ff8464b3b2a

# Operator 执行 HOME/WORKSPACE 隔离

## 变更

- `ExecutionKind::Operator` 执行获得按执行的隔离布局：`<data_dir>/operator/<id>/{home,workspace}`（`crates/worker/src/layout.rs` 预留，`operations/create.rs` 准入通过后物化）。
- `crates/worker/src/operations/operator_env.rs`：冻结配置快照（明文含 provider api key）以 0600 写入执行 home；`resolve()` 双门（快照 + workspace 目录同时存在）失败回退节点级默认，不影响 Maintenance/Agent/Brain。
- fresh 会话在 how_append 后注入 HOME 覆盖对（envs 尾部、经 harness envs 持久化），resume 由 `resume.rs` 重建 env_passthrough，重启后 HOME/cwd 重建。
- core `Config::load_with_home`：候选链（`.opencoder/config.json`、XDG、domain 文件）重定向到执行 home；None 时与普通 `load` 等价。
- web：`AppState.config_home` 穿参 drain 栈（`handle/drain.rs` `DrainContext`），prompt/config 载入均走 `load_with_home`。
- `crates/worker/src/brain/workdir.rs`：`session_dirs()` 统一裁定执行目录（Operator→workspace，缺失回退 node workdir）；`workloads/agent.rs` `create_session` 显式携带 workdir。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 快照物化一次性 + 0600 | `materialize_writes_snapshot_once_and_keeps_permissions_private` | `crates/worker/src/operations/operator_env.rs` |
| resolve 双门 + Operator-only | `resolve_requires_snapshot_and_is_operator_only` | `crates/worker/src/operations/operator_env.rs` |
| HOME 覆盖对 | `env_pairs_point_home_at_the_execution_home` | `crates/worker/src/operations/operator_env.rs` |
| 快照经 load_with_home 回读 | `snapshot_round_trips_through_config_load_with_home` | `crates/worker/src/operations/operator_env.rs` |
| api key 保留 | `snapshot_keeps_provider_api_key` | `crates/worker/src/operations/operator_env.rs` |
| 候选链/域文件重定向 | `load_with_home_redirects_global_candidates_and_domain_files` | `crates/core/src/config/tests.rs` |
| None 等价普通 load | `load_with_home_none_matches_plain_load` | `crates/core/src/config/tests.rs` |
| O7 全链路（cwd=workspace、HOME=执行 home、CONFIG_OK、快照 0600、无泄漏、respawn 后 follow-up 重建、4 次 LLM 请求、SSE stream_end） | `operator_execution_isolates_home_and_workspace` | `tests/operator_e2e/isolation.rs` |

- 全量回归（分批 `cargo test` 覆盖全部 24 个 workspace 成员，等效 `cargo test --workspace`）：5362 passed / 2 failed。
  - `opencoder-agent` `three_runtime_versions_keep_live_model_calls_and_global_fifo`：高负载（load 100+，localhost 连接 79s）超时 flake，单独复跑 32.16s 通过。
  - `opencoder-worker` `brain_graph::an_unconfirmed_v3_run_never_moves_to_another_node_or_plans_offline`：干净 HEAD（2ae28fbc worktree 验证）同样失败，基线即坏，与本迭代无关。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → exit 0 零警告。
- fmt：本次触及文件 `cargo fmt --check` 全部干净。
- 行数 gate：`operator_env.rs` 245 行、`isolation.rs` 152 行、`brain/workdir.rs` 126 行，均 ≤400。
