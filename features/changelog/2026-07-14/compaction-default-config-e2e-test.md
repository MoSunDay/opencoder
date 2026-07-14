# 压缩默认配置端到端验证测试

## 背景

`compaction_and_model.rs` 已有 7 个测试覆盖压缩的各个切面（token 估算触发、reserved 缩窗、small_model 用于摘要、reported_tokens 用 input_only、disabled 跳过、单用户多工具轮次触发、model 切换生效）。但缺少一个**端到端**测试：在不覆盖任何 compaction 配置字段的情况下（即使用 `Config::default()` 的真实默认值 context_limit=128_000 / threshold=80_000 / reserved=20_000 / tail_turns=2），验证压缩确实会被触发并实际执行（footprint 缩减、summary 注入、tail 消息保留）。此前所有测试都用 `base_config()` 构造 session 但未断言默认值本身，也没有走到 `run()` 验证压缩真正发生。

## 变更

### 新增测试（`crates/session/tests/compaction_and_model.rs`，+87 行）

- **`compaction_fires_with_real_default_config`**：纯测试新增，零源码改动。
  - **前置断言**：`base_config()` 产出的 session 持有真实默认值（128k / 80k / 20k / 2），预算 = min(80_000, 128_000−20_000) = 80_000。
  - **Layer 1 — 触发层**：三条 200k 字符 user 消息（~150k 估算 token）远超 80k 预算，`should_compact(&s)` 在任何 `run()` 调用前即返回 `true`。
  - **Layer 2 — 执行层**：用 `MockChatClient`（summary 脚本 + default done）调 `run(&mut s, "go")`，断言：
    - footprint 缩减 >50%（`chars_after < chars_before / 2`）；
    - 首条消息为 synthetic compaction summary（`synthetic == true`，前缀 `[Conversation summary so far]`）；
    - u1/u2 被摘要移除，u3 保留在 tail 内；
    - mock 收到 ≥2 次请求（summary 调用 + turn 调用）。
  - 确定性：MockChatClient + tempdir，无时序/并发/网络依赖。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 压缩在真实默认配置下端到端触发并执行 | `compaction_fires_with_real_default_config` | `session/tests/compaction_and_model.rs` |

## Gate

> 以下数据为**仅暂存 `compaction_and_model.rs`** 的干净状态（stash 掉 5 个范围外文件后）实跑结果，与提交范围一致。

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 523 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

注：工作树中另有 5 个范围外文件（`core/src/tool.rs`、`core/tests/tool_filter.rs`、`session/src/runner.rs`、`tui/src/render.rs`、`tui/src/render_tests.rs`）属另一在途任务，不含本次提交范围。含这些文件时全量测试为 530 passed / 0 failed。
