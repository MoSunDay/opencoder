# subagent tool-set 受 tools_subagent 能力门控 + 应用 stash@{1}（validation-stash-non-tools-subagent）

## 背景

仓库本地积累了 4 条 stash，均为历史会话遗留的待合并工作。以 `features/changelog/` 为 ground truth
对四者逐一裁决，避免盲目 apply 带来回归或重复：

| stash | 裁决 | 依据 |
|-------|------|------|
| `stash@{1}` `validation-stash-non-tools-subagent` | ✅ **应用**（唯一真新功能） | base=58c7542；功能为 subagent 能力门控 |
| `stash@{0}` `validation-isolate-divergent` | 已被 @{1} 包含 | `git diff @{0} @{1} -- agent.rs` = 0 行（agent.rs 字节相同） |
| `stash@{2}` WIP tui composer | 被 HEAD 取代 | HEAD 已有 `wrap_rows`+`delete_word_back`（grep=20），@{2} 二者皆无 → 应用即回归 |
| `stash@{3}` rename+payload | 已由并发会话正式入库 | flusher 测试以 `8d0a01d` 落地；rename / bash_guard / selection 均已定稿于 HEAD |

最终只 apply `@{1}`，2 处冲突解决（runner.rs 假冲突 keep-ours；model_menu/view.rs 加 ProviderList 早返回分支）。

## 变更

### subagent 能力门控（headline）
- **`crates/session/src/runner.rs:657`**：新增 `fn valid_subagent_options(plan, tools_on)` —— 把
  subagent tool-set 的可用性收敛到 `tools_subagent` 能力开关之后。
- **`crates/session/src/runner.rs:696`**：dispatch 路径读 `parent.config.capabilities.tools_subagent_enabled()`
  并经 3 个 call site（:703/:713/:721）传入 `valid_subagent_options`，能力关时 subagent 工具集不可派发。

### model_menu provider-list 弹窗
- **`crates/tui/src/model_menu/view.rs`**：新增 `render_provider_list_popup`，ProviderList 状态走早返回分支
  （冲突解决取 theirs）。

### 资产：内建 skills
- **`crates/core/assets/skills/{submit,review,repo-local-memory,do-and-done}/`**：随 stash 捆绑入库的
  仓库本地工作流 skill 文档（均 ≤298 行，见 review SKILL 描述）。

### 缓存相关（细节见同目录既有文档）
- `cache_salt` 配置 + `OPENCODER_CACHE_SALT` 环境覆盖 + 三态测试 → 见
  [cache-salt-env-override-test.md](cache-salt-env-override-test.md)。
- `Usage` 结构体新增 `cache_read_tokens`/`cache_creation_tokens`，经 SQLite `usage_json` 往返不丢 → 见
  [usage-cache-tokens-persistence.md](usage-cache-tokens-persistence.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| tools_subagent 能力开 → subagent 可从 act 派发 | `tools_subagent_is_dispatchable_from_act` | `crates/session/tests/capabilities_and_tools.rs` |
| tools_subagent 能力关 → 拒绝派发 | `tools_subagent_rejected_when_capability_disabled` | `crates/session/tests/capabilities_and_tools.rs` |
| tools_subagent 序列化往返 | `tools_subagent` round-trip 断言 | `crates/session/tests/capabilities_and_tools.rs` |
| cache_salt 出站请求体三态 + 子 agent 隔离 | `cache_salt.rs`（3 test） | `crates/session/tests/cache_salt.rs` |

- 全量回归：`cargo test --workspace` → 718 passed / 0 failed / 0 ignored（61 二进制）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- 新文件行数：最大 298（repo-local-memory/SKILL.md）≤ 400

## Impact Surface
- 新增 `tools_subagent` 能力开关：默认行为**不**派发 subagent tool-set，需显式置 `tools_subagent=true`。
  对未设该能力的现有配置：subagent 工具集不再无条件可派发（更安全的默认）。
- `Usage` 结构体扩字段：消费该结构体的测试字面量需补 `..Default::default()`（本提交已修
  `capabilities_and_tools.rs` 与并发落地的 `event_sink_flusher.rs`）。
- 不影响：session 协议、store 编码格式（usage_json 向后兼容增字段）、CLI/Web 入口。

## Related Docs
- [cache-salt-env-override-test.md](cache-salt-env-override-test.md)
- [usage-cache-tokens-persistence.md](usage-cache-tokens-persistence.md)
- [event-write-batching-model-warn.md](event-write-batching-model-warn.md)
