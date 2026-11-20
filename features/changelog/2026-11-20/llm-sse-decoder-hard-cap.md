# LLM SSE 解码缓冲加 64MiB 硬上限，无终止符字节流超限即终止

一句话：上游持续推送不含帧终止符的字节时，SSE 解码器的待定缓冲不再
无限增长，超过 64MiB 硬上限立即按终态失败终止流，防止节点 OOM。

## 背景与缺陷

- `crates/llm/src/sse.rs` 的 `push` 无条件 `buf.extend_from_slice(chunk)`，
  只有 `drain()` 在收到终止符（`\n\n` 等）后才消费缓冲；帧内 JSON 解析、
  工具调用累计等均发生在终止符之后。
- 两条流式链路复用该解码器：`client/chat_stream.rs`（chat completions）
  与 `client/responses_stream.rs`（Responses API）。
- 两处的空闲看门狗都只在**解码出数据帧**时重置——只送字节、不送帧的上游
  永远不会触发看门狗，也不受 HTTP 读超时约束（字节一直在到达）。于是一个
  退化或恶意的上游可以在 600s 空闲窗口内把无终止符缓冲灌到 GB 级，拖垮
  节点内存。

## 变更清单

- `crates/llm/src/sse.rs`
  - 新增 `pub const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024`（64MiB
    待定字节硬上限）与错误类型 `SseOverflow { limit }`（实现
    `Display`/`Error`，消息为 "SSE frame exceeded {limit} bytes without a
    terminator"）。
  - `SseDecoder` 存储 `limit`；新增 `with_limit(limit)` 构造器，
    `new()` = `with_limit(MAX_PENDING_BYTES)`，`Default` 语义不变。
  - `push` 签名改为 `push(&mut self, chunk: &[u8]) ->
    Result<(), SseOverflow>`：当 `buf.len() + chunk.len() > limit` 时返回
    `Err`（缓冲保持不动，不追加该 chunk），正常路径行为不变（等于上限
    的边界值仍接受）。
- `crates/llm/src/client/chat_stream.rs` — push 返回 `Err` 时按本文件
  中途致命条件（如 invalid SSE JSON）的同款分类映射为 `OnceError::Connect`
  （终态、不重试），错误消息含 overflow 内容，且不再继续读流。
- `crates/llm/src/client/responses_stream.rs` — push 返回 `Err` 时映射为
  `Failure::Terminal(anyhow!("{overflow}"))`，与既有 `Failure` 用法一致。
- sse.rs 既有单元测试的 `push` 调用点补 `.unwrap()`；新增 4 个上限行为
  用例。
- 本 changelog。

超限分类为**不可重试**的理由：无终止符的无限字节属于协议层损坏而非瞬态
中断，重试只会重新拉一遍同样的垃圾流。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 超限返回 Err 且携带 limit | `unterminated_bytes_over_limit_error_out` | `crates/llm/src/sse.rs` |
| 上限内大帧正常 drain 且缓冲复位 | `frames_under_limit_still_drain` | `crates/llm/src/sse.rs` |
| 等于上限的边界值接受 | `boundary_exact_limit_accepted` | `crates/llm/src/sse.rs` |
| 默认解码器使用 64MiB 上限（不实际分配） | `default_decoder_uses_max_pending_bytes` | `crates/llm/src/sse.rs` |

实际命令与结果（仅定向，未运行全量测试）：

```text
cargo clippy -p opencoder-llm --all-targets -- -D warnings   # exit 0，零警告
cargo test -p opencoder-llm --lib sse                        # 26 passed / 0 failed
cargo test -p opencoder-llm --lib stream                     # 5 passed / 0 failed
cargo test -p opencoder-llm --test stream_timeout            # 2 passed / 0 failed
cargo test -p opencoder-llm --test stream_retry              # 5 passed / 0 failed
cargo test -p opencoder-llm --test connect_retry             # 2 passed / 0 failed
cargo test -p opencoder-llm --test responses                 # 16 passed / 0 failed
```

- 后四个集成测试覆盖两条被改调用点的流式链路（chat 重试/超时与 Responses
  SSE），确认正常终止符流行为不变。
- 行数上限：`sse.rs` = 473 行（上限 800）。
- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或全站
  UI 验收。

## 回滚

- revert 本次提交即可；无 schema、配置或依赖迁移。回滚后恢复旧行为：
  无终止符上游可使待定缓冲无限增长直至 OOM。
- 回滚时删除本 changelog。
