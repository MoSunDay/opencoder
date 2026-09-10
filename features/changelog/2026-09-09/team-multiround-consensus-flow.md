Commit: (working-tree, 基于 b465f440)

# team 多轮共识闭环：对齐子轮与 next_question 轮间提示的端到端契约测试

补齐 team 话题循环在 deploy 拓扑下的多轮语义覆盖：单轮内队长判定未对齐后仅对歧义成员追派对齐子轮（`alignment` prompt 判别串、结果 `kind="alignment"`、其余成员不重派），closing 的 `complete=false` 裁决把 `next_question` 作为提示注入下一轮 plan prompt（判别串“上一轮建议的下一问题：…”），下一轮对齐后 `complete=true` 收束。

新增 `crates/worker/tests/platform/team_multiround_consensus.rs`（163 行），复用 `Fleet` mock 基建（真实 control app + WS worker 节点 + `MockChatClient` 脚本队列），12 条脚本按消费顺序驱动 2 轮决策链：plan(t1)→arch/qa 作答→summary 未对齐(qa 歧义)→qa 对齐追答→summary 对齐→closing 继续(带 next_question)→plan(t2, 提示注入)→arch/qa 作答→summary 对齐→closing 完成。三层断言：

- 执行层：`settled`→`done`、节点会话索引含 `member-` 前缀；`requests()` 精确 12 条（无隐式额外 LLM 调用），第 5 条含“需要你针对性澄清”与歧义问题原文，第 8 条含轮间提示原文，第 2 条含 t1 问题。
- 状态层：topic `team.json` `status=finished`、`finish_reason=complete`、`final_summary` 相符；`turns` 恰 2 条且字段逐一相等（turn 记账 1 起始：`{turn:1,aligned:true,sub_turns:2}`、`{turn:2,aligned:true,sub_turns:1}`）。
- 产物层：`1/0/{arch,qa}/result.json` `kind=answer`、`1/0/summary.json` `aligned=false` 且 `ambiguities[0].node_id=qa`、`1/1/qa/result.json` `kind=alignment`、`1/1/arch/` 不存在（仅重问 qa）、`1/1/summary.json` `aligned=true`、`2/plan.json` question 与 closing(t1) 的 next_question 逐字一致、`2/1/` 不存在（第二轮首轮即对齐）。

勘误：实施时确认磁盘 turn 目录与 `turns[].turn` 均为 1 起始（`layout::validate_turn` 上界含 1），计划稿中的 0 起始写法已按代码为准修正。`tests/platform/main.rs` 挂载模块，无生产代码改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 多轮 team 执行（2 轮 + 1 对齐子轮）→ done | `team_multiround_consensus_runs_alignment_subturn_and_next_round_hint` | `crates/worker/tests/platform/team_multiround_consensus.rs` |
| 对齐子轮仅重派歧义成员 + alignment prompt 判别串 | 同上 | 同上 |
| closing next_question → 下轮 plan 提示注入 | 同上 | 同上 |
| turn 记账（aligned/sub_turns/participants）与 topic 终态 | 同上 | 同上 |
| 全轮产物落盘契约（result/summary/plan 逐文件） | 同上 | 同上 |

## 验证结果

- 全量 `cargo test --workspace` 终跑复验：**4993 passed / 0 failed / 0 ignored**（361 个测试目标；较初跑 4987/5 ignored 的差异来自并行会话在途用例增补与解除 ignore）。初跑曾现 `opencoder::daemon_smoke` 单例偶发失败，单跑、初跑重跑与终跑复验均绿，与本次改动无关——本次仅新增 worker 平台测试。
- 定向：`cargo test -p opencoder-worker --test platform` → 10 passed（含新用例）；`-p opencoder-worker -p opencoder-team -p opencoder-store` → 104 passed / 0 failed。
- `cargo clippy -p opencoder-worker -p opencoder-team --all-targets` 与 `cargo clippy --workspace --all-targets`：均零警告（原阻断 workspace 级 clippy 的并行会话 `session/bash.rs` 在途修改已解除，复验通过）。
- 行数 gate：新增文件 163 行（≤400）；无硬编码凭据。
