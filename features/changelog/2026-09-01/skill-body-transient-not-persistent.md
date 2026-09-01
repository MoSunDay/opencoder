Commit: (working-tree, skill 正文持久化注入改为 armed 期瞬时携带)

# skill 正文不再永久驻留 transcript——一次性生命周期收口

## 背景

`skill_context::ensure_full_body_loaded` 把激活 skill 的正文作为**持久化**
`synthetic=true` user 消息写进 transcript 并落库（`[skill loaded] <path>` 标记
+ 全文）。run 结束时 `clear_on_run_end` 只清掉内存 skill、`sessions.skill` 行、
tail reminder 与 latent 解锁，**正文消息却永远留在 transcript 里**：之后该
session 的每一次模型输出（后续 run 的每一轮请求）都在重新提交 skill 全文，
直接违背 `skill_lifecycle` 自己声明的一次性契约（"subsequent runs start
skill-less"）。集成复现：run1 结束（skill 已清）后，run2 纯文本请求仍携带正文。

## 变更摘要

- 正文投递改为**瞬时 per-call**：新增纯函数
  `skill_context::transient_body_message`（同步、无副作用、不落库），在
  `runner/llm_call.rs` 中与 tail reminder 同一接缝 append 在 payload 末尾——
  仅当 skill 处于 armed 状态（Primary、非 workflow、有 Source 路径、正文非空）
  时，该 run 的每一轮请求携带一次；run 结束 skill 清除后自动消失，无需删除
  任何已持久化内容。消息文本保留 `[skill loaded] <path>` 标记块（模型侧格式
  不变）与 `[INCOMPLETE SKILL]` 截断续读契约。
- 删除持久化机制全套：`ensure_full_body_loaded`、`loaded_marker_matches`、
  `marker_paths` 及 run_loop 每轮注入点；compaction 折叠后重注入、resume marker
  扫描随之不需要（armed 期每轮重新派生，天然免疫折叠/恢复）。
- `tail_reminder` 不再扫描 transcript：`[active skill]` 指针仅在
  armed-but-空正文退化场景出现（正文在相邻 payload 中时指针冗余，与既有
  marker 抑制行为一致）。
- 超 20K token 正文的整行截断 + `offset=` 续读语义原样保留。

## 兼容性

- 提交时解析（`$name` token → discovery → 正文合成）仍恰好一次，未变。
- 模型可见内容不丢：armed 期间每轮请求都带全文（无状态协议要求），run 结束
  后按一次性契约不再提交——修复点正是「跨 run 永久重复提交」。
- 无 schema/迁移变化；旧 session 里已落库的 `[skill loaded]` 历史消息按普通
  历史保留，不再被 marker 扫描特殊对待。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| armed 期每轮请求携带正文恰好一次、零持久化 | `body_rides_every_armed_round_and_is_never_persisted` 等 | `crates/session/tests/skill_body_injection.rs`（重写） |
| run 结束后后续 run 零正文零 tail | contract (c) 扩展用例 | `crates/session/tests/skill_tail_cleared_after_run_end.rs`、`skill_one_shot.rs` |
| 超 20K 截断 + `[INCOMPLETE SKILL]` 续读 | 截断用例 | `crates/session/tests/skill_body_injection.rs` |
| 复合 `$A $B` 标记块 canonical + 各自 Source 注解 | 复合用例 | `crates/session/tests/skill_body_injection.rs` |
| subagent/workflow/无 skill 门禁 | 门禁用例 | `crates/session/src/skill_context.rs`（in-file tests 重写） |

- 回归：`cargo test --workspace` 结果见下（多会话共享工作树，门禁为全树合并结果）。
- clippy（touched crates `--all-targets`）：零告警。
