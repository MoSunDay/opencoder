# stream_retry 满载竞态修复：Conn::Reset 以 chunked 畸形帧钉死 ChunkError

## 背景

rules/02 全量回归门闭合过程中，`cargo test --workspace --no-fail-fast` 在满载下出现
1 例非确定性红：`opencoder-llm` 的
`retry_exhaustion_emits_single_non_doubled_error`（`crates/llm/tests/stream_retry.rs`）
断言终态 Error 文案含 `chunk read error`，满载跑出的却是
`"stream failed: idle timeout after 3 attempts"`。隔离重跑恒绿（9/9）。

## 根因

该套件的 `make_client` 经 `new_with_read_timeout(..., READ_TIMEOUT=1s, ...)` 构造，
而 client（`crates/llm/src/client.rs::new_with_read_timeout`，刻意设计）把**事件级
idle watchdog 与字节级 read timeout 绑定为同值**（均为 1s）。用例原用 `Conn::Stall`
（发一帧后静默 hold 2s）——静默瞬间两个 1s 定时器同刻武装，满载下调度先後不定，
中断成因在 `ChunkError`（read timeout 先到）与 `IdleTimeout`（watchdog 先到）之间
非确定翻转。断言硬编码了 `chunk read error`，故为计时竞态 flake，非产品缺陷。

## 修复（仅测试侧，零产品代码改动）

- `Conn` 新增 `Reset { delta }` 变体：以 `Transfer-Encoding: chunked` 提供响应体，
  先发一个合法 chunk（含 `delta` 帧），随后发**畸形 chunk-size 行**
  （`not-a-chunk-size\r\n`）——hyper 的 body 读取在该行确定性报错，零定时器参与，
  必然走 `StreamInterruption::ChunkError` 路径。
- 原 flaky 用例的 3 × `Conn::Stall` 改为 3 × `Conn::Reset`，原断言
  （恰一个 Error、单前缀、含 `chunk read error`、`after 3 attempts`）**原样保留**，
  无降级。
- mock server 的 SSE 头写入从无条件前置改为各 arm 自行写入（Reset arm 用
  chunked 头变体 `write_sse_header_chunked`）；其余 arm 行为不变。
- 干净 EOF（原 `Truncate`）与静默 hold（原 `Stall`）语义不受影响：现有
  `truncated_stream_retries_then_completes` / `chunk_error_retries_then_completes` /
  `idle_heartbeat_retries_then_completes` 照旧覆盖各自路径。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 确定性 ChunkError 耗尽路径单 Error/单前缀 | `retry_exhaustion_emits_single_non_doubled_error` | `crates/llm/tests/stream_retry.rs` |
| Stall→ChunkError 恢复路径（未动，回归确认） | `chunk_error_retries_then_completes` | 同上 |
| Heartbeat→IdleTimeout 恢复路径（未动，回归确认） | `idle_heartbeat_retries_then_completes` | 同上 |

- 定向：`cargo test -p opencoder-llm --test stream_retry` → 5 passed / 0 failed（连跑 3 次全绿）
- 全量回归（rules/02 门，本次闭合）：`cargo test --workspace --no-fail-fast`
  → 197 个 result 汇总 / **3100 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（`Finished`）
- build：`cargo build --workspace` → 零错误

## 关联

- 补记：`features/changelog/2026-08-19/autopilot-verify-tighten-and-review-cancel.md`
  的「全量门阻塞于并发迭代」缺口自本次起闭合（并发迭代已落 `6a5e3f3`，中途态消除）。
