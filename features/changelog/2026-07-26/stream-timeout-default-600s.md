Commit: (working-tree, pre-initial-commit)

# fix(core,llm): 流式超时两层默认值统一为 600s（10 min idle-reset）

## 背景

`stream_idle_timeout_secs`（语义层，idle-reset：只要有任意流式内容即重置计时器）
默认 120s，而字节层超时（`DEFAULT_READ_TIMEOUT` 与 `build_http_client` 的
`read_timeout`）默认 300s。两层默认不一致导致：若仅放宽语义层到 600s，纯停滞
场景下字节层仍会在 300s 抢先触发，使 600s 配置形同虚设。本轮把两层默认统一
为 **600s**，让「idle 容忍 10 分钟」在所有场景一致生效。

语义不变：仍是 idle-reset —— 只要 stream 持续交付数据，计时器在每个 chunk 上
重置，永不中断；只在「连接活着但长时间零内容」时才判 stalled。

## 变更

### 语义层默认（`crates/core/src/config.rs`）
- `stream_idle_timeout()` 访问器 `unwrap_or(120)` → `unwrap_or(600)`（config.rs:382）。
- 字段文档注释「Defaults to 120」→「Defaults to 600」。

### 字节层默认（`crates/llm/src/client.rs`）
- `DEFAULT_READ_TIMEOUT` `Duration::from_secs(300)` → `from_secs(600)`（client.rs:32）。
- 文档注释「5 minutes」→「10 minutes」。

### HTTP 客户端默认（`crates/core/src/net.rs`）
- `build_http_client` 默认 `read_timeout` `from_secs(300)` → `from_secs(600)`（net.rs:57）。
- 文档注释「300s」→「600s」。（net.rs 仅改默认实参与注释，无值断言测试，故无需
  改测试。）

**为何两层一起改**：若只放宽语义层（600s）而字节层保持 300s，纯停滞场景下
字节层会 300s 抢先触发，使 600s 配置形同虚设。两层统一才能让「idle 容忍 10
分钟」在所有场景一致生效。

## 测试覆盖

值断言测试随默认值同步更新（rename + assert 值）：

| 文件 | 测试名 | 断言 |
|------|--------|------|
| `crates/core/src/config.rs` | `stream_idle_timeout_defaults_to_600s` | `Config::default().stream_idle_timeout() == 600s` |
| `crates/llm/src/client.rs` | `default_read_timeout_is_600s` | `DEFAULT_READ_TIMEOUT == 600s`（regression guard，防误改回小值） |

## 回归结果（rules/02-regression-gate）

- `cargo test --workspace` → **1176 passed / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- （clippy `--all-targets` 已编译全部 target，覆盖 build gate）
