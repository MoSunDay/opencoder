# 冗余清理：无用依赖、死代码与重复实现收敛（零功能变更）

纯清理提交：删除无调用者的依赖与死代码、收敛重复的错误构造与测试助手。
所有改动 wire 级等价；`crates/event`（有意预留地基）与各契约模块不动。

## 改动口径

- **A. 依赖清理**（cargo 1.100 unused 警告 + grep 双重确认）：
  根 `Cargo.toml` 删 6 个无引用 workspace 条目（`glob`、`regex`、
  `opencoder-agent/cli/event/server`）与 `workspace.package.authors`；
  crate 级删 `llm/thiserror`、`session|web|local/dirs`、`event/tracing`；
  `control/tokio-stream` 移 dev-dependencies（仅 tests 使用）。
- **B. 死代码删除**：`tui/model_menu/headers.rs` 测试 `left()`（与
  `tests/common.rs` 同名真用例无关）、`session/tools/bg.rs BgEntry.session_id`
  （级联收敛 `register/register_supervised` 及全部调用方）、
  `session/mcp/pool.rs McpConnection.client`、`tui/mcp_menu McpEntry.env`；
  `local/ts/tmux.rs tmux_id` 的过期 `#[allow(dead_code)]` 移除（字段有读取者）。
- **C. web 错误家族收敛**：`api.rs` 五个 `error_4xx/5xx` 收敛为
  `impl Into<String>` 泛型 + 私有 `error_body`，收编 `error_503`；
  `api_events/inputs/meta/ops/subagents` 本地副本删除，`api_project_util`
  改 re-export。**7 个被 control `#[path]` 挂载的文件保持本地实现**——
  两族错误体形状不同（见 control 索引），去重会改变节点 API wire 输出。
- **D. control 转发收敛**：`api/mod.rs` 四个实现泛型化后，
  `api/project_util.rs` 的 4 个委托 wrapper 改 `pub use` re-export；
  消息字符串逐调用点核对字节不变。
- **E. 测试助手收编**（rules/03）：`wait_for_subagent_start` ×3 →
  `session/tests/common`；todos 新建 `tests/support`（`wait_until_running`
  ×2、`wait_for_session_message` ×2，措辞/文档差异统一取保守侧）；
  `HangingStream` ×3 → `web/tests/support/hanging_stream.rs`（通道容量
  1 vs 128 统一为 128——永不发送，容量不参与行为）；根
  `running_mode_switch_e2e.rs` 本地 `http()` 改用 `tests/support/http_util`。
- **F. 文档**：control 模块索引补记 `#[path]` 挂载与两侧错误体形状差异的
  去重禁区（本次清理中实际踩到的坑）。

## 测试覆盖

| 改动 | 定向测试 | 结果 |
|------|----------|------|
| A+B 依赖与死代码 | `cargo check --workspace`；clippy 定向 8 crate；session `--test bg_kill_all`、lib `tools::bash`/`mcp::pool`；tui lib `model_menu|mcp_menu`；local lib `ts` | 零警告；1+22+8+131+91 全过 |
| C web 错误家族 | `cargo test -p opencoder-web`（312 过 / 3 挂起前已存在） | 等价 |
| D control 收敛 | `cargo test -p opencoder-control`（lib 149；e2e 261/262，1 项 stash 复跑证实基线同败） | 等价 |
| E 测试助手 | session 3 个 test 目标（2+2+2）；todos 3 个（8+2+1）；根 `--test running_mode_switch_e2e`（2）；web 3 个（7+1+1，`agent_switch_wins_mutex…` 基线即抖动） | 全过 |
| 合并态复核 | `cargo check --workspace`、clippy `--tests` 8 crate、`cargo test -p opencoder-control --lib`（149/149） | 零警告 |

## 实测记录

- `cargo check --workspace` → Finished，零警告
- `cargo clippy [--tests] -p opencoder-web -p opencoder-control -p opencoder-session -p opencoder-todos -p opencoder-tui -p opencoder-local -p opencoder-llm -p opencoder-event` → 零警告（仅依赖 crate `opencoder-dag-runtime` 既有 1 条，未触及）
- 上表各项定向测试均实跑通过；失败项均以「stash 本次 diff 基线复跑同败」证实为既有问题，未追改
- 62 个跟踪文件 +205/−631，新增 2 个测试支撑文件（68+43 行），净删约 315 行
- 测试范围：本次修改及其直接影响链路，未运行全量测试
