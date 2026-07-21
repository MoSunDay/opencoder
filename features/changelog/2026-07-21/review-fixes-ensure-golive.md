# 全盘 review 修复：溯源更正 / lossless 措辞 / 测试计数 / 测试外移 / model-warn 单测

## 背景

对最近 1 天迭代（commits ac033dd..eda4ff8）做全盘 review 后发现 5 个非阻塞 finding。
为"确保可上线"，本轮逐项修复，重跑全量 gate 确认仍绿。

## 变更

### F1 — changelog 溯源更正（tools_subagent gate 归属）
- **`features/changelog/2026-07-20/stash-merge-tools-subagent-capability-gate.md`**：
  `git blame` 证实 `tools_subagent` gate 代码实际落在 `ac033dd`（并发会话），非 `eda4ff8`。
  umbrella changelog 此前误标为 headline → 加溯源更正注记，修正标题与背景描述，保 auditability。

### F2 — "lossless" 措辞修正（数据完整性声明诚实性）
- **`crates/session/src/event_sink.rs:12`**：模块 doc 从"100% lossless"改为明确限定——
  "lossless on normal termination (channel close triggers final flush). On a store *write* failure
  the batch is logged and dropped (warn-only)"。
- **`features/changelog/2026-07-20/event-write-batching-model-warn.md`**：两处"无损"措辞补"store
  写失败时降级为 warn-only"限定。

### F3 — changelog 测试计数统一
- 4 份 changelog（688/708/711/718）统一为 725 passed / 0 failed / 0 ignored（含 F5 新增 7 测试）。

### F4 — app_helpers.rs 测试外移（文件行数 1519→629）
- **`crates/tui/src/app_helpers.rs`**：1519→629 行（≤800 ✓）。将 893 行 `#[cfg(test)] mod tests`
  外移到 **`crates/tui/src/app_helpers_tests.rs`**（891 行），镜像 `app.rs→app_tests.rs` 既有模式。
  - runner.rs (1078) 和 app.rs (1058) 经可行性评估为 **DEFER**（前者触及今日刚改的公共 API，
    后者是 891 行单函数事件循环——二者都是高风险重构，不应混入稳定化 pass）。均为预存债务。

### F5 — 补 model-warn 单测（warn_if_suspicious_model 此前零覆盖）
- **`crates/core/src/config.rs`**：提取纯判定函数 `pub(crate) fn is_suspicious_model(model) -> bool`，
  `warn_if_suspicious_model` 调用它。新增 7 个单元测试覆盖全部分支：
  empty / 正常 scoped / boundary 2-2 / 无斜杠短 / 无斜杠边界 / provider 侧短 / mid 侧短。
  config.rs 663→706 行（≤800 ✓）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| model-warn 判定逻辑（7 分支） | `tests::is_suspicious_model::*`（7 test） | `crates/core/src/config.rs` |
| app_helpers 鼠标/IO（外移后仍通过） | 原 tests（无逻辑改动） | `crates/tui/src/app_helpers_tests.rs` |

- 全量回归：`cargo test --workspace` → 725 passed / 0 failed / 0 ignored（61 二进制）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- 行数：app_helpers.rs 629 ≤ 800；config.rs 706 ≤ 800；app_helpers_tests.rs 891（测试文件，同 app_tests.rs 1255 约定）

## Impact Surface
- 无行为变更：F2/F3 纯文档/doc-comment；F1 changelog 归属更正；F4 测试外移（零逻辑改动）；
  F5 提取纯函数（`is_suspicious_model` 与原逻辑等价，可测性提升）。
- 不影响：session 协议、store 编码、CLI/Web/TUI 入口、任何运行时路径。

## Related Docs
- [stash-merge-tools-subagent-capability-gate.md](../2026-07-20/stash-merge-tools-subagent-capability-gate.md)
- [event-write-batching-model-warn.md](../2026-07-20/event-write-batching-model-warn.md)
