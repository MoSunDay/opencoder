Commit: (working-tree, pre-initial-commit)

# web go-live review：SSE 幻影 handle 修复 + 默认回环 + 稳定 data_dir 哈希

## 背景
web 逻辑 go-live review 发现三个问题：
1. **F1（正确性）** `get_events` 用 `map.entry().or_insert_with()` 会插入一个无 drain 任务的 `SessionHandle`；随后 `admit_and_drain` 用「handle 存在」当作「drain 在跑」，导致客户端先订 SSE 再发首条 prompt（或 drain 结束后重订再发）时 `need_spawn=false` → prompt 持久化但**永不处理**。
2. **F2（安全）** `Serve` 子命令 `host` 默认 `0.0.0.0`，无鉴权（已记为 non-goal）→ LAN 内任何人可触发工具/bash、读 workdir、耗 LLM 额度。
3. **F3（可靠性）** `data_dir_for` 用 `std::collections::hash_map::DefaultHasher` 派生 DB 路径身份；std 明确不保证其跨版本稳定 → 工具链升级可能让同一 workdir 映射到新目录「丢」会话。

## 变更

### A — 解耦 drain 运行态与 handle 存在性（F1）
- `SessionHandle` 增 `draining: AtomicBool`；`cancel` 由 `CancellationToken` 改为 `Mutex<CancellationToken>`（每次 spawn 刷新新 token，避免上次 interrupt 的永久取消毒化后续 drain）。
- `admit_and_drain`：get-or-create handle（共享 broadcast 通道，早订阅者仍能收到 live）→ `draining.swap(true)` CAS 决定 spawn，**不再依赖 map 存在性**。
- `drain_to_completion`：新增 `DrainGuard`（Drop 复位 `draining`，含 panic 路径）；完成后**保留 handle 于 map**（供 late SSE replay + re-admit），仅 resume 失败（session 行缺失）时移除。
- `api.rs`：`get_events` 改用 `SessionHandle::new()`；`post_interrupt` 改 `h.cancel.lock().await.cancel()`。

### B — 默认绑回环（F2）
- `crates/cli/src/lib.rs`：`Serve { host }` 默认值 `0.0.0.0` → `127.0.0.1`。
- `crates/cli/src/serve.rs`：`serve_launch` host `0.0.0.0` → `127.0.0.1`。

### C — data_dir 稳定哈希（F3）
- `crates/web/src/lib.rs`：`hash_of` 由 `DefaultHasher` 改为无依赖 FNV-1a 64（身份键、非安全场景）；新增 `#[cfg(test)]` pin 到固定 hex `832ee9edd819d93b`，防未来算法漂移。

## 涉及文件
- `crates/web/src/handle.rs` — `SessionHandle`/`admit_and_drain`/`drain_to_completion` 重构 + `DrainGuard`
- `crates/web/src/api.rs` — `get_events` 句柄构造 + `post_interrupt` cancel 互斥
- `crates/web/src/lib.rs` — FNV-1a `hash_of` + pin 测试
- `crates/cli/src/lib.rs`、`crates/cli/src/serve.rs` — 默认 host 回环
- `crates/web/tests/web_contract.rs` — 两处内联 `SessionHandle` 构造补字段 + 新增回归测试

## 测试覆盖

drain 生命周期契约独立到 `crates/web/tests/web_drain_contract.rs`（HTTP CRUD/SSE/switch 留在 `web_contract.rs`），两文件均 ≤400 行（规则 03）。

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 早订阅 handle 不阻塞 drain（F1 回归） | `pre_existing_events_handle_does_not_block_drain` | `web_drain_contract.rs` |
| drain 完成后再次 prompt 再 spawn（draining 复位 / DrainGuard） | `second_prompt_after_drain_completion_spawns_fresh_drain` | `web_drain_contract.rs` |
| interrupt 后再 prompt 跑到完（cancel token 每 spawn 刷新） | `prompt_after_interrupt_runs_to_completion` | `web_drain_contract.rs` |
| 先订 /events 再 prompt 收 live 帧（共享通道） | `events_subscriber_before_prompt_receives_live` | `web_drain_contract.rs` |
| POST /prompt 配置加载失败 → 结构化 500（错误路径） | `post_prompt_returns_500_on_malformed_config` | `web_drain_contract.rs` |
| /events 慢订阅者背压：lag 丢弃不阻塞、最近帧仍送达 | `events_stream_survives_subscriber_lag` | `web_drain_contract.rs` |
| hash_of 跨版本稳定 + pin（F3） | `hash_of_is_stable_and_pinned` / `hash_of_distinguishes_paths` | `crates/web/src/lib.rs` |
| interrupt 取消 token（重构后） | `interrupt_cancels_running_drain_token` | `web_contract.rs` |
| SSE replay+live | `sse_replays_persisted_events_then_live` | `web_contract.rs` |

- 全量回归：`cargo test --workspace` → 293 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- fmt：`cargo fmt --all --check` → 干净

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy workspace | clean | clean |
| fmt | clean | clean |
| test (web) | 9 | 9 contract + 6 drain + 2 unit |
| 全量 test | 285 passed | 293 passed |
| 测试文件行数 | web_contract 514（超软 400） | web_contract 382 / web_drain 378（均 ≤400） |

## 相关文档
- [agents/web](../../../agents/web/index.md) — 关键抽象与主流程已同步
