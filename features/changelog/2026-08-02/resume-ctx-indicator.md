# TUI 状态栏 ctx% 指示器：resume 后反映真实流水账（修复恒零 bug）

## Summary

resume（`--continue`/`--session`）或压缩后 `TranscriptReset` 重建出的
`ChatView.context_used` 恒为 0——状态栏的 `ctx%` 只显示系统提示词 token
（`sys_tokens`），无论历史多长都看不到累积。根因：`replay_into_chat`
（`session_ui.rs`）只重建显示用的 `ChatBlock`，从不触碰 `context_used`。

修复：返回前用 `estimate_messages(messages)` 一次性算出整段消息流水的 token 量，
与权威压缩路径 `compaction::estimated_tokens` 同源（减去 system 部分，system 由
`sys_tokens` 在渲染时单独叠加）。一处改动同时修复 resume 与 live `TranscriptReset`
两条路径。

> 注：压缩**触发决策**不受此 bug 影响——它直接遍历 `session.messages`，一直正确。
> 仅显示用的指示器失真。

## Changes

### `crates/tui/src/session_ui.rs`
- `replay_into_chat` 返回前补 `chat.context_used = estimate_messages(messages) as u64;`
- 新增 `use opencoder_llm::estimate_messages;`（已在 `opencoder_ll` crate root re-export）

### `crates/tui/tests/resume_context_used.rs`（新建，136 行）
- 三条回归测试，覆盖 resume 后 `context_used` 的正确性。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| resume 后 ctx% 反映真实流水账 | `resume_context_used_matches_transcript_estimate` | `crates/tui/tests/resume_context_used.rs` |
| 空历史 → 0 | `resume_context_used_empty_when_no_messages` | 同上 |
| 消息越多 ctx% 越大（单调性） | `resume_context_used_grows_with_more_messages` | 同上 |

## 全量回归

- 全量回归：`cargo test --workspace` → **1621 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`session_ui.rs` 800（迭代 ≤800）；`tests/resume_context_used.rs` 136（新增 ≤400）

## 备注

- `/task` 的 mode 显示经确认为最新持久化值（每次打开即时查库，所有 mode 切换路径均
  经 worker `persist_session_agent` 落库），无需改动。
- live 路径中 user 消息 token 不被增量计入是独立的预存小问题（不影响压缩触发），
  本次不在范围内。
