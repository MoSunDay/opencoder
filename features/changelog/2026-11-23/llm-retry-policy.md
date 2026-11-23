# LLM 连接重试预算提升至 10 次并统一 5 分钟单次等待上限

一句话：`crates/llm/src/retry.rs` 的流式连接重试预算由 5 次提升到 10 次
（1 次初始 + 9 次重试），保持 0.5 s 起步的指数退避；新增 `BACKOFF_MAX_MS`
（300_000 ms）硬上限，计算退避与服务端 `Retry-After` 提示均受 5 分钟封顶，
任何单次等待都不会超过 5 分钟。

变更明细：

- `MAX_ATTEMPTS` 5 → 10：pre-stream 连接/HTTP 状态重试预算；中流重试
  `MAX_STREAM_ATTEMPTS = 3` 不变（每次中流重试都会丢弃部分响应重生成，预算刻意收紧）。
- 指数退避曲线：`BASE_BACKOFF_MS * 2^(attempt-1)`，10 次尝试对应
  0.5/1/2/4/8/16/32/64/128/256 s。
- 新增 `BACKOFF_MAX_MS = 300_000`：`backoff_millis` 与 `backoff_duration`
  （含 jitter）均 `min` 该上限，防止预算外推或极端 attempt 值突破单次等待上限。
- `RETRY_AFTER_MAX_SECS` 120 → 300：与 `BACKOFF_MAX_MS` 对齐；
  `retry_delay` 取「计算退避 / 封顶后的服务端提示」较大者，两侧均 ≤ 5 分钟，
  故单次等待严格 ≤ 5 分钟。
- `crates/llm/src/client/transport.rs` 注释中的 UI 展示示例同步为 `retry n/10`；
  `crates/llm/tests/connect_retry.rs` 断言改用 `MAX_ATTEMPTS` 常量，避免再次硬编码。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 10 次预算与指数退避曲线 | `backoff_millis_doubles_each_attempt` | `crates/llm/src/retry.rs` |
| 单次等待 5 分钟封顶（含 jitter） | `backoff_is_capped_at_five_minutes` | `crates/llm/src/retry.rs` |
| 退避 = 纯退避 + ≤250 ms jitter（1–10 次） | `backoff_duration_within_millis_plus_jitter` | `crates/llm/src/retry.rs` |
| Retry-After 提示 300 s 封顶 | `retry_delay_caps_server_hint_at_max` | `crates/llm/src/retry.rs` |
| 第 10 次仍失败即终止（off-by-one 哨兵） | `retry_decision_retryable_retries_then_fails_at_max` | `crates/llm/src/retry.rs` |
| 预算内每次可重试错误都重试 | `retry_decision_sequence_recover_on_third_attempt` | `crates/llm/src/retry.rs` |
| 连接重试事件携带真实预算 `max` | `retry_after_header_delays_retry_then_completes` | `crates/llm/tests/connect_retry.rs` |

- 定向回归：`cargo test -p opencoder-llm --lib retry` → 15 passed / 0 failed；
  `cargo test -p opencoder-llm --test connect_retry` → 2 passed / 0 failed；
  `cargo test -p opencoder-llm`（全部 target）→ 195 passed / 0 failed。
- Clippy：`cargo clippy -p opencoder-llm --all-targets -- -D warnings` → 零警告；
  `cargo fmt -p opencoder-llm -- --check` → 无差异。
- 测试范围：本次修改及其直接影响的链路（llm crate 全部 target）；未运行全量测试。
