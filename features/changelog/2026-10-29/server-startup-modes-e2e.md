# opencoder-server 启动面进程级覆盖补齐与孤儿测试清理

一句话：新增根包进程级套件 `tests/server_modes_e2e/`（真二进制 + loopback，零外部服务）覆盖 CLI 拒绝矩阵、`--token-file`/env-only token/`--auth-enabled false`/`--metrics-token-file`/`--web false` 真启动冒烟、`--resources`/`--todo-resources` 的 `serve()` loopback 冒烟、schedules.json 启动路径一次性种子导入与 SIGTERM 优雅停机（SSE reconnect + 干净退出）；顺带修复 `--web` 旗标无法关闭 SPA 的装配缺陷，并清理 `tests/operator_e2e/` 的孤儿文件（其依赖的沙箱 run_mode 派发能力已随早前重构移除），以现行契约重写为活跃轮次 steer/queue 输入用例。

## 变更清单

- `crates/server/src/main.rs`：`--web` 改为显式布尔（`ArgAction::Set`，对齐 `--auth-enabled` 的既有模式）；此前为 SetTrue 且默认 true，`--web=false` 是用法错误，`serve_release` 的 web 裁剪分支从 CLI 不可达。默认行为不变。
- `agents/server/index.md`：登记 `--web true|false` 语义。
- `tests/operator_e2e/sandbox_inputs.rs`：删除（孤儿文件：`main.rs` 未声明、依赖的 `write_card`/`prepare_rootfs`/run_mode 派发已移除，从不参与编译）。
- `tests/operator_e2e/active_inputs.rs`（新）：按现行 admission 契约重写该场景（`admitted_seq` 幂等重放、载荷冲突 409、steer 晋升下一轮、queue 独立成轮、完成后重放不触发新轮），`main.rs` 挂载为 O8。
- `tests/server_modes_e2e/`（新目录）：`main.rs`/`proc.rs`（进程装配：环境清洗、就绪行/健康探测、RAII 清理）/`cli_reject.rs`/`token_modes.rs`/`serve_modes.rs`/`shutdown.rs`。
- `tests/support/fleet_proc.rs`：`Fleet::server_pid()`（+8 行，信号场景使用）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 启动拒绝矩阵（互斥/缺参/空 token 文件/metrics==server/无凭据）11 例 | `invalid_startup_flags_fail_fast_before_binding` | `tests/server_modes_e2e/cli_reject.rs` |
| `--token-file` 启动并强制 Bearer | `token_file_boot_enforces_bearer` | `tests/server_modes_e2e/token_modes.rs` |
| 仅 `OPENCODER_SERVER_TOKEN` 环境变量启动 | `env_only_token_boots` | `tests/server_modes_e2e/token_modes.rs` |
| `--auth-enabled false` 免鉴权 + `/api/auth/status` | `auth_disabled_serves_without_bearer` | `tests/server_modes_e2e/token_modes.rs` |
| metrics token 只开 `/metrics`，不开 `/api/*`（401） | `metrics_token_scoped_to_metrics_only` | `tests/server_modes_e2e/token_modes.rs` |
| `--web true` 保持 SPA 与 API 挂载 | `web_flag_keeps_spa_and_api_mounted` | `tests/server_modes_e2e/token_modes.rs` |
| `--web false` 裁剪 SPA（`/`、`/static` 非 200），API 不受影响 | `web_disabled_drops_spa_keeps_api` | `tests/server_modes_e2e/token_modes.rs` |
| `--resources` serve() loopback + role 字段 | `resources_service_serves_loopback_health` | `tests/server_modes_e2e/serve_modes.rs` |
| `--todo-resources` serve() loopback + role 字段 | `todo_resources_service_serves_loopback_health` | `tests/server_modes_e2e/serve_modes.rs` |
| schedules.json 启动路径一次性导入（二启不重复） | `startup_seed_imports_schedules_json_once` | `tests/server_modes_e2e/serve_modes.rs` |
| SIGTERM：SSE reconnect 帧 + 干净退出 | `sigterm_drains_open_sse_and_exits_cleanly` | `tests/server_modes_e2e/shutdown.rs` |
| 活跃轮次 steer/queue 输入：幂等、冲突 409、单次晋升 | `active_round_inputs_steer_queue_idempotent_and_promoted` | `tests/operator_e2e/active_inputs.rs` |
| `--web` 显式布尔解析（默认开，非法值报错） | `web_defaults_on_and_takes_an_explicit_boolean` | `crates/server/src/main.rs` |

## 定向回归

- `cargo test -p opencoder --test server_modes_e2e` → 11 通过。
- `cargo test -p opencoder --test operator_e2e -- active_inputs` → 1 通过；整个 target 13 例中 11 过、2 败（`fork`、`isolation`）经 pristine HEAD 复跑确认为仓库既有失败，与本次改动无关。
- `cargo test -p opencoder-server` → 6 通过（含新增解析测试）。
- `cargo clippy -p opencoder-server / -p opencoder-control --all-targets`、`cargo clippy -p opencoder --tests` → 0 警告。
- 未运行全量测试。
