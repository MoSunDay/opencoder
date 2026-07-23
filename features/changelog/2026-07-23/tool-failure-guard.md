Commit: (working-tree, pre-initial-commit)

# feat(session): tool-failure threshold guard with exponential backoff

## 背景
当某个工具持续失败（例如外部资源不可用、参数错误），drain 主循环会陷入
无意义的重试循环：模型反复调用同一个失败工具，消耗 token 却无法推进。
已有的 doom-loop 守卫（`DOOM_THRESHOLD=3`）按 `name:input` 签名匹配，只
能拦截「签名完全相同」的重复；当模型每次微调入参（签名变化）但仍调用同一
失败工具时，doom 守卫无法识别，循环继续。

本次新增一个**按工具名**计数的失败守卫：同一工具名连续失败到阈值即中止
当前 turn，并在每次失败间施加指数退避，给外部资源恢复时间、降低无谓重试。

## 变更
### 配置（core）
- **`crates/core/src/config.rs`**：新增 `ToolGuardConfig` 结构（
  `max_consecutive_failures=3`、`backoff_base_ms=200`、`backoff_max_ms=2000`），
  带各自 `serde(default=...)` 与 `Default` 实现；`Config` 新增 `tool_guard`
  字段（`#[serde(default)]`），保证旧配置文件可平滑反序列化。
- **`crates/core/src/lib.rs`**：re-export `ToolGuardConfig`。

### 运行时（session）
- **`crates/session/src/tool_guard.rs`**（新文件）：纯函数实现。
  - `record(map, name, is_error, cfg) -> (tripped, backoff)`：成功清零计数，
    失败递增；达到阈值返回 `tripped=true`，并按 `base * 2^(n-1)`（封顶
    `max`）计算退避；阈值为 0 时整体禁用。
  - `backoff(count, cfg)`：指数退避，位移量 `.min(20)` 防溢出。
  - `worst(map)`：返回计数最高的 `(name, count)`，用于诊断信息。
- **`crates/session/src/lib.rs`**：`pub mod tool_guard`。
- **`crates/session/src/runner.rs`**：`run_loop` 内维护每轮 `FailureMap`，
  在工具结果批次后调用 `record`（runner.rs:508-528）记录每条结果并施加
  最大退避 `sleep`；任一工具触发阈值（runner.rs:568-576）则发射
  `SessionEvent::Error` 并 `break` 中止当前 turn。

### 测试
- **`crates/session/src/tool_guard.rs`**：7 个单元测试（计数重置、阈值精确
  触发、指数退避与封顶、工具间独立计数、成功插入重置、阈值为 0 禁用、
  `worst` 空映射）。
- **`crates/session/tests/tool_failure_guard.rs`**（新文件）：4 个集成测试，
  覆盖「3 次连续失败即停」「触发时发 Error 事件」「成功插入重置计数」
  「禁用守卫可无限失败」。每个 fixture 用不同 `input` 绕开 doom-loop 签名
  匹配，使失败计数按工具名累积。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 成功重置计数 | `success_resets_counter` | tool_guard.rs |
| 阈值精确触发 | `threshold_trips_exactly_at_limit` | tool_guard.rs |
| 指数退避与封顶 | `backoff_exponential_and_capped` | tool_guard.rs |
| 工具间独立计数 | `independent_tools_tracked_separately` | tool_guard.rs |
| 阈值为 0 禁用 | `zero_threshold_disables_guard` | tool_guard.rs |
| 3 次失败即停 | `threshold_stops_after_three_consecutive_failures` | tests/tool_failure_guard.rs |
| 触发发 Error 事件 | `emits_error_event_on_threshold` | tests/tool_failure_guard.rs |
| 成功插入重置 | `success_between_failures_resets_counter` | tests/tool_failure_guard.rs |
| 禁用守卫无限失败 | `disabled_guard_allows_unlimited_failures` | tests/tool_failure_guard.rs |

- 全量回归：`cargo test --workspace` → 全绿（隔离 target dir 复跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`crates/session/src/tool_guard.rs` ≤ 400（新文件）

## Impact Surface
- 用户：模型反复调用同一失败工具时，连续 3 次后该 turn 自动中止并发
  Error 事件，避免无意义重试与 token 浪费；退避降低对失败资源的冲击。
- 配置：新增可选 `tool_guard` 段，默认值即旧行为之外的新增保护；设
  `max_consecutive_failures=0` 可完全禁用，行为退回改动前。
- 不影响：store / web / cli / drain 的 steer/queue 语义、doom-loop 守卫
  （两者互补：doom 按 `name:input` 签名，本守卫按工具名）。

## Related Docs
- [agents/session](../../agents/session/index.md)
- [rules/01-mandatory-tests.md](../../../rules/01-mandatory-tests.md)
