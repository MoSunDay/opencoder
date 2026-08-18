Commit: (working-tree, post-860831d)

# skill 激活后持久注入全文（>20K token 截断 + 续读提示）

## Context

skill 激活后，模型只能从尾部 transient 提醒里拿到 SKILL.md 的**路径**，还得花一次 `read` tool call 才能拿到正文——每个激活的 skill 都多一轮往返。方案 A：激活的 skill 在每个 LLM 调用前幂等地把「路径 + 全文」作为一条 `synthetic=true` 的**持久** user 消息注入历史（只注一次、经 `record()` 落库、resume 可恢复），尾部提醒降级为兜底指针。`skill_prompt` / `> Source:` 前缀 / latent 工具解锁机制均不动。

## Change Summary

- `crates/session/src/skill_context.rs`（净 +198）：
  - `const MAX_INJECT_TOKENS = 20_000`（`llm::estimate` token 单位，与 read 工具 5K 截断、compaction 阈值同源）。
  - 纯函数 `full_body_marker(path)`（`[skill loaded] {path}`）与 `loaded_marker_matches(messages, path)`（扫 `synthetic && user && starts_with(marker+"\n")`，带换行防 `/a/SKILL.md` 误匹配 `/a/SKILL.md.bak`；先例 `compaction/mod.rs` 摘要 marker 扫描）。
  - `injectable_body(body, path)`：≤20K 原文返回；超限按**整行**二分取最大合规前缀（estimate 随行数单调），截断点后追加 `[INCOMPLETE SKILL] truncated at ~20K tokens; {n} lines remain; read the rest with the read tool: read(path="{path}", offset={next_line}).`——`offset` 为截断点后一行（1-based，与 read 工具对齐，可链式续读），文案镜像 `tools/read.rs` 的 `[INCOMPLETE READ]`。零行合规（超大单行）时退化为仅提示（offset=1）。
  - `pub async fn ensure_full_body_loaded(&mut SessionState)`：门控与 `tail_reminder` 一致（仅 Primary、排除 `workflow`）；legacy 无 `> Source:` 前缀的 body 不注入；剥掉 `> Source:` 前缀块后注入 `marker 行 + 空行 + injectable_body`（剥前缀使截断 offset 与 `read(path, offset)` 的真实文件行号对齐）；marker 未命中才 `Message::user` + `synthetic=true` + `record()`（幂等、落库）。
  - `reminder_text` 激活段改为指向已加载消息 + 兜底 read（「已作为 `[skill loaded]` 消息加载到上文；找不到（如压缩后）再 read」）。
- `crates/session/src/runner/mod.rs`（+5）：run_loop 中 compaction 检查后、`LlmRoundStart` 前调用 `ensure_full_body_loaded(session).await`——单点覆盖全部 5 个激活入口（TUI `$` picker、`$name` 内联解析、queue/steer 消费时激活、autopilot review_pass、resume 恢复 skill）。compaction 把 marker 折叠进摘要 → 下一轮自动重注入（预期兜底闭环成本）。
- 不动：`core/skill.rs`、`llm_call.rs` latent 解锁、`skill_resolve.rs` 的 `SKILL_TRIGGER`、TUI（synthetic 消息本就不渲染）。

## Validation（当次实跑）

- `cargo test -p opencoder-session --lib`：**374 passed / 0 failed**（含 skill_context 9 项：原 4 + 新 5）。
- `cargo test -p opencoder-session --test skill_body_injection`：**5 passed / 0 failed**（新建）。
- `cargo test -p opencoder-session --test skill_context_tail --test skill_mid_run --test skill_queue_drain --test compound_cmd --test autopilot_review --test skill_resume --test steer_skill_deferral --test plain_skill_prompt --test autopilot_skill_persist`：**34 passed / 0 failed**。
- `cargo test --workspace`：**2943 passed / 0 failed**（基线 2855 → ↑，无 #[ignore] 无删测试；工作树含并发会话在途 diff，`plan_phase.rs` 瞬态编译错误出现后自行消解）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `skill_context.rs::injectable_body_small_is_verbatim` | unit | 预算内原文返回 |
| `skill_context.rs::injectable_body_exactly_at_budget_is_verbatim` | unit | 边界含 20,000（80,000 chars） |
| `skill_context.rs::injectable_body_truncates_on_whole_lines_with_continuation_notice` | unit | 行级截断、前缀 ≤20K、`1 lines remain` + `offset=5`、行 0..=3 留 / 行 4 弃 |
| `skill_context.rs::injectable_body_single_oversized_line_degrades_to_notice_only` | unit | 超大单行零行合规 → 仅提示 + offset=1 |
| `skill_context.rs::full_body_marker_and_scan_semantics` | unit | marker 格式、路径前缀碰撞防护、非 synthetic/user 不计、空集 |
| `skill_body_injection.rs::small_skill_body_rides_payload_and_persists` | integration | 首轮 payload 含 marker+全文、落 store 可 round-trip、幂等不重注、system 字节两轮稳定、tail 不含正文 |
| `skill_body_injection.rs::oversized_skill_body_truncates_with_continuation_notice` | integration | 截断版落库、notice 精确文案、截断行不入 payload |
| `skill_body_injection.rs::switching_skills_injects_new_entry_keeps_old` | integration | 换 skill 注新条目、旧条目 append-only 保留、双 body 同乘后续 payload |
| `skill_body_injection.rs::subagent_and_workflow_never_get_injection` | integration | explore/workflow 排除（transcript + payload 双断言） |
| `skill_body_injection.rs::legacy_body_without_source_prefix_is_not_injected` | integration | 无 `> Source:` 前缀不注入 |
| `skill_context_tail.rs::active_skill_source_path_rides_tail_reminder_and_keeps_system_clean`（+1 断言） | integration | 激活段新文案指向 `[skill loaded]` 消息 |

设计取舍：频繁切换 skill 时旧注入 append-only 保留（≤20K/份）以保证 resume 语义简单；`estimate` 为近似值（chars/4），行级累加保证不超限但可能略低于 20K。
