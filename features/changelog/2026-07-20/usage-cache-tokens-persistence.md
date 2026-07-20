# 完整 provider usage 持久化：捕获 cache_read / cache_creation token

## 背景

此前 opencode 只把 LLM 响应里的三件套写进 `messages.usage_json`：

```json
{"input_tokens":..,"output_tokens":..,"total_tokens":..}
```

prompt-cache 的 token（Anthropic `cache_read_input_tokens`、网关 `cache_read`、OpenAI 嵌套 `prompt_tokens_details.cached_tokens` 等）在
`crates/llm/src/client.rs::parse_usage` 解析阶段就被**直接丢弃**——全代码库 grep
`cache_read/cache_write/cached_tokens` 命中为 0。也就是说：

- 完整 provider usage 只在 `run_stream` 栈内存里活了几毫秒，响应解析完即丢失；
- `messages.usage_json` 永远只有 3 字段；
- 历史 cache 数据**已无法找回**（没有 jsonl、没有原始请求日志，`JsonlStore` 仅用于单向旧迁移，运行时无人调用）。

结论：要让"cache 也能统计到"成立，**必须改 opencode 核心**去记录完整 usage。

## 选型：方案 B1（最小改动，复用现有持久化）

在三条候选里选定 **B1**：

| 方案 | 做法 | 取舍 |
|------|------|------|
| A（不改核心） | sync 只读现有 3 字段 | 与现网对不上（cache_read 常是大头），统计偏低 ❌ |
| B2（jsonl 全量） | 在 `run_stream` 落 `requests.jsonl` | 保留完整原始负载，但要新增并发追加/轮转/行号游标，更 invasive |
| **B1（推荐，采纳）** | 扩 `Usage`/`MessageUsage` + `parse_usage` 解析 cache，顺现有 `usage_json` 落库 | 改动最小，天然走现有持久化，无需 schema 迁移，cache 从此不再丢 ✅ |

B1 不引入新文件层；sync 工具直接读 `messages.usage_json`（现在变完整），增量游标逻辑完全不变。
**代价**：历史数据仍是 3 字段，无法回填——只能从此刻起向前追踪。

## 变更

### `Usage`（`crates/llm/src/event.rs`）
新增两个字段，均带 `#[serde(default)]`：

```rust
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,       // 命中缓存、便宜计价的 token
    #[serde(default)]
    pub cache_creation_tokens: u64,   // 本轮写入缓存的 token
}
```

字段命名是**我们自己的持久化规约**（不是 provider 原始 key），serde 直接按此名落 `usage_json`，
下游消费者（sync/billing）从此处读稳定 key。doc 注释写明 provider 命名差异与历史数据不可回填。

### `parse_usage`（`crates/llm/src/client.rs`）
重写为：先抽三件套，再把 provider 各路 cache 命名**归一化**进两个字段。
新增私有 helper `first_u64(obj, &[keys])` 按优先级返回首个命中值。归一规则：

| 归一字段 | 接受的 provider 原始 key（按优先级） |
|----------|--------------------------------------|
| `cache_read_tokens`     | `cache_read_input_tokens` → `cache_read` → `prompt_tokens_details.cached_tokens`（OpenAI 嵌套） |
| `cache_creation_tokens` | `cache_creation_input_tokens` → `cache_write` |

- 显式 Anthropic key 优先于短别名（确定性优先级）；
- 全部缺失 → 0（向后兼容旧 OpenAI / 无 cache 场景）。

### `MessageUsage`（`crates/core/src/message.rs`）
镜像两个新字段，同样 `#[serde(default)]`。这是实际写进 `messages.usage_json` 的类型。
`#[serde(default)]` 是关键：旧行（3 字段 JSON）反序列化时缺失的 cache 字段取 0，
`row_to_message` 的 `unwrap_or_default()` 兜底——**无需任何 schema 迁移**。

### `core_usage`（`crates/session/src/runner.rs`）
手动 `Usage`→`MessageUsage` 转换函数补拷两个新字段（无 `From` impl，此处是唯一接缝）。

### 持久化路径（无改动）
`messages::append` / `append_many` / `import` 继续用 `serde_json::to_string(&msg.usage)` 通用序列化，
新字段自动出现在 `usage_json` TEXT 列里。读取侧 `row_to_message` 同样通用反序列化。

## 设计备忘（供 sync 工具 / 后续迭代）

- **数据源**：`messages.usage_json`（TEXT，每条 assistant 消息一行）。
- **规约 key**：`cache_read_tokens` / `cache_creation_tokens`（u64，缺失即 0）。
- **回填**：不可能——历史行无此字段，读出为 0。统计口径应标注"自 <上线时间> 起"。
- **schema 版本**：未升版；`usage_json` 是灵活 JSON，加 key 无需迁移。
- **provider 覆盖**：Anthropic / 多数 OpenAI 兼容代理（含 GLM）/ 网关短别名 / OpenAI 原生嵌套，
  四种命名均已覆盖并有测试。
- **未做（明确排除）**：不在 `run_stream` 落 jsonl 全量请求/响应（那是 B2，本次不采纳）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| OpenAI 三件套解析 + 无 cache → 0 | `parse_usage_reads_openai_base_fields` | `crates/llm/src/client.rs` |
| Anthropic `cache_read_input_tokens`/`cache_creation_input_tokens` | `parse_usage_reads_anthropic_cache_fields` | `crates/llm/src/client.rs` |
| 网关 `cache_read`/`cache_write` 短别名 | `parse_usage_reads_gateway_cache_aliases` | `crates/llm/src/client.rs` |
| OpenAI 嵌套 `prompt_tokens_details.cached_tokens` | `parse_usage_reads_openai_nested_cached_tokens` | `crates/llm/src/client.rs` |
| 显式 key 优先于短别名（确定性优先级） | `parse_usage_prefers_explicit_anthropic_key_over_alias` | `crates/llm/src/client.rs` |
| 仅 base 字段（向后兼容）cache 归 0 | `parse_usage_missing_cache_fields_default_to_zero` | `crates/llm/src/client.rs` |
| 空 usage 对象全 0 不 panic | `parse_usage_empty_object_is_all_zeros` | `crates/llm/src/client.rs` |
| `first_u64` 优先级与缺失返回 None | `first_u64_returns_first_present_key` | `crates/llm/src/client.rs` |
| cache token 经 SQLite `usage_json` 往返不丢 | `append_and_load_preserves_all_roles_and_blocks`（新增 cache 断言） | `crates/store/tests/store_integration.rs` |

`parse_usage` 此前**零覆盖**，本次一并补齐（规则 01）。

## 附带修复（dirty-tree 预存 clippy nit，非 B1 范围）

为过规则 02 clippy gate，顺带修了工作树里他人未提交的机械 clippy 提示（均语义等价、一行级）：

- `crates/core/src/config.rs`：`std::env::var(..).ok()` → `if let Ok(..)`；`.or_insert_with(ProviderConfig::default)` → `.or_default()`。
- `crates/session/src/tools/mod.rs`：`CapabilitiesConfig` 字段重赋值 → 内联 `..Default::default()`。
- `crates/session/tests/cache_salt.rs`：`.map_or(false, ..)` → `.is_some_and(..)`。

## Gate

| 项 | 结果 |
|----|------|
| `cargo build --workspace` | 零错误 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo test --workspace` | 708 passed / 0 failed / 0 ignored |
