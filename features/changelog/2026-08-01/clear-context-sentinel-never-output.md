# fix(session/tui/cli): /act_clear_context 的哨兵标记永不输出、永不进模型 context

## 背景

`/act_clear_context` 在 `handoff_plan` 中持久化内部哨兵
`<<OPENCODER_CLEAR_CONTEXT_MARKER>>`（resume 靠它区分 clear-context 边界
与 plan→act handoff，从而重建 fresh-start marker）。但该哨兵此前会泄露到
用户可见输出：TUI `replay_into_chat` 把 `handoff_plan` 渲染成 Plan 卡片时
直接显示原始哨兵文本；`session show --json` 也原样 dump。

## 变更

- `crates/session/src/control_cmd.rs`：新增公共谓词
  `is_clear_context_handoff(&str) -> bool`（唯一真源，替代各显示层硬编码）。
- `crates/session/src/lib.rs`：re-export `is_clear_context_handoff`。
- `crates/session/src/resume.rs`：哨兵判断改走公共谓词（行为不变）。
- `crates/tui/src/session_ui.rs`：`replay_into_chat` 对哨兵 `handoff_plan`
  跳过 Plan 卡片渲染——原始哨兵永不出现在 UI。
- `crates/cli/src/session_cmd.rs`：`build_session_json` 将哨兵 `handoff_plan`
  抹为 `None`（`handoff_seq` 仍保留边界信息），`session show --json` 不再输出。
- 模型 context 本已安全（resume 把哨兵转换为 fresh-start marker 后才重建
  transcript），新增回归测试钉死该不变量。

## 测试覆盖

| 文件 | 测试名 | 断言 |
|------|--------|------|
| `crates/session/src/control_cmd.rs` | `clear_context_sentinel_predicate` | 谓词对哨兵为真、对真实 plan/空串为假 |
| `crates/session/tests/control_cmd.rs` | `clear_context_sentinel_never_reaches_model_context` | clear 后继续对话：唯一 LLM 请求体不含哨兵字符串，且含 fresh-start marker |
| `crates/tui/tests/plan_card_full_flow.rs` | `clear_context_sentinel_renders_no_plan_card` | `handoff_plan` 为哨兵时 `replay_into_chat` 不渲染任何 Plan 块、不输出哨兵 |
| `crates/cli/src/session_cmd.rs` | `build_session_json_redacts_clear_context_sentinel` | JSON 输出不含哨兵，`handoff_plan` 抹为 None，`handoff_seq` 保留 |

## 全量回归

- 全量回归：`cargo test --workspace` → **1587 passed / 0 failed / 1 ignored**（当次实跑；ignored 为既有 `research_smoke_bing_wikipedia`，需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`control_cmd.rs` 317、`resume.rs` 716、`session_cmd.rs` 467（迭代 ≤800）；`tests/control_cmd.rs` 392、`tui/tests/plan_card_full_flow.rs` 234（迭代 ≤800）
