Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# agent-team 话题推进合约记忆 + e2e 场景回归

## 背景

团队讨论的合约口径此前散落在 runtime 注释与零散单轮用例里，缺一条「主持人按话题推进交付」的端到端回归：主持人是唯一规划者——开话题、点名参与者、轮内未对齐点名追答、对齐后收束进入下一话题；全部话题完成后拼接各话题 `final_summary` 得最终结论。成员始终 act 模式作答（自由文本），只有主持人产决策 JSON。本次不改生产代码，只把合约固化为记忆与双场景回归。

## 变更

- `agents/team/index.md` 新增「话题推进合约」段：主持人唯一规划者、成员 act 作答、话题内 1..N 轮 / 轮内 0..N 对齐子轮（sub ≥1 仅拉回未对齐成员，`RESULT_ALIGNMENT`）、对齐才记轮并进入 closing。
- 新增 `crates/team/tests/topic_contract_flow.rs`：真实注册节点（arch-captain + pay-backend/sre/risk-ctl，带能力快照）+ `MockDispatcher` per-node FIFO 全脚本 19 次 prompt（主持人 10 决策 + 成员 9 次 act 作答），驱动两个话题：
  - 话题1《回调超时与降级》：turn1 三人作答未对齐 → 仅点名 backend/risk 追答（sub1，sre 不被拉入）→ 对齐 → turn2 参与者收缩为 backend/risk → closing 完成；
  - 话题2《重试与幂等》：1 轮对齐 → closing 完成。
- 新增 `crates/web/tests/team_topic_sequence.rs`：HTTP e2e 复现同一两话题场景（复用 `api_teams.rs` harness 形态），经 `POST /api/teams` 建队 → 依次 `POST .../topics` → 轮询 detail 至 finished。

## Impact Surface

- 新增 `crates/team/tests/topic_contract_flow.rs`（324 行）、`crates/web/tests/team_topic_sequence.rs`（349 行）
- 修改 `agents/team/index.md`
- 生产代码零改动

## 测试覆盖

| 契约 | 用例 | 文件 |
| --- | --- | --- |
| 两话题均 complete；turns 形态 2轮(sub 2/1)+1轮；参与者收缩 | `captain_progresses_two_topics_and_members_stay_in_act_mode` | `crates/team/tests/topic_contract_flow.rs` |
| 追答只拉回点名成员（kind=alignment，sre 无目录、无幻影第三子轮） | 同上 | 同上 |
| 追答 prompt 含「需要你澄清 + 渠道真实 P99」；主持人 10 次调用全含「只输出 JSON」、成员 prompt 不含 | 同上 | 同上 |
| sre 恰 2 次调用且 topic 分别指向两个话题；总派发 19 次 | 同上 | 同上 |
| 台账按 (topic,node) 去重 4+3 行全 finished；已完成话题重跑零派发 | 同上 | 同上 |
| 最终结论拼接两份 final_summary（2.4s/放行/重试 3 次/幂等键） | 同上 | 同上 |
| HTTP e2e：detail 树 turns/sub_turns/results/plan、跨团队 `/api/topics` 2 话题、share 磁盘布局、最终结论 | `team_topic_sequence_drives_two_topics_to_a_final_conclusion` | `crates/web/tests/team_topic_sequence.rs` |

## Validation

- `cargo test -p opencoder-team --test topic_contract_flow` 1 passed
- `cargo test -p opencoder-web --test team_topic_sequence` 1 passed
- `cargo test -p opencoder-team` 全绿（7 套件 31 项）；`cargo test -p opencoder-web --test api_teams` 10 passed
- `cargo clippy -p opencoder-team -p opencoder-web --all-targets -- -D warnings` 零警告
- 全量回归 Gate（交付时）：`cargo test --workspace -j 12 --no-fail-fast -- --test-threads=4` 402 个测试结果块，除 `opencoder::running_mode_switch_e2e`（并行会话负载抖动，隔离重跑通过，与本变更无关；本次零生产代码改动）外全绿

## Related Docs

- [团队模块](../../../agents/team/index.md)
