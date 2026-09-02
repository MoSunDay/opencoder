Commit: (working-tree)

# skill 正文交付收敛为一次性：仅激活后首轮 payload 携带，后续轮次零携带

## 背景

用户反馈：skill 激活时附加的正文应当在**整个 run 中只出现一次**，但实际观察
到 skill 正文在**每一轮 LLM 请求**都重复出现——`runner/llm_call.rs` 对每个
armed 轮次都在 payload 末尾重新派生并附加 transient `[skill loaded]` 正文消
息（不进 transcript，所以只能逐轮重附）。一个 20K token 的 skill 跑 10 轮
就是 10 份全价重复输入；且每份都位于 transcript 之后的 payload 尾部，位置
逐轮漂移，永远落在 provider prompt-cache 可命中前缀之外，缓存完全失效。

## 实现（crates/session，13 文件 +318/−221）

- **交付门**：`SessionState` 新增私有 `skill_body_delivered: Arc<Mutex<bool>>`
  ——一次性交付台账。`set_skill` 每次写入（新激活）都复位为 false；
  `resume.rs` 构造时初始 false（crash-mid-run 恢复的 skill 在 resumed run
  首轮重新交付一次）。
- **`skill_context::deliver_body_once(&SessionState) -> Option<Message>`**：
  门 + 交付一体——首见未交付的 armed 正文时返回 `[skill loaded] <path>`
  marker 块 + 正文消息并翻转门；同 run 后续轮次一律返回 None。复用原
  `body_and_pointer` 的全部 gate（Primary 且非 workflow、`> Source:` 路径、
  空 body 退化为 pointer）与 20K token 截断/`[INCOMPLETE SKILL]` 续读机制。
- **`llm_call.rs`**：per-call 附加改为 `deliver_body_once` 调用——**轮次
  2..N 的 payload 不再携带任何 skill 正文**；需要回看时模型按 marker 的源
  路径 `read` SKILL.md（marker 本身就是指路牌）。
- **`transient_body_message` → `body_message`** 更名（纯构造，语义从"每轮
  瞬时"改为"一次性交付"）；compaction `estimated_tokens` 只在交付门未消耗
  时计入正文（首轮 payload 超预算守卫保持成立）。
- `skill_lifecycle::clear_on_run_end` 契约不变（内存 + store 行双清）；
  正文从不落库，run 结束后零残留。`[active skill]` tail reminder 保持
  per-call fallback 语义（仅 armed 但 body 为空的退化场景出现）。

## Token 效果

同一激活生命周期内：正文只出现在第一次 LLM 调用（trailing、1 份），后续每轮
payload 的 skill 正文为 **0 份**（旧实现每轮 1 份全价、且不可缓存命中）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 一次性交付门（首轮交付、二次 None、新激活重臂、排除面） | `deliver_body_once_is_one_shot_per_activation` | `crates/session/src/skill_context.rs` |
| 契约：首轮 trailing 恰一次、轮次 2..N 零正文、不落 transcript/store、run 后无残留 | `armed_body_ships_once_then_stops_and_is_never_persisted` | `crates/session/tests/skill_body_injection.rs` |
| 复合 `$A $B`：单轮排序 marker 块 + 合并正文、次轮清空 | `compound_body_keeps_inner_annotation_and_one_sorted_marker_block` | `crates/session/tests/skill_body_injection.rs` |
| mid-run 激活：首个观察轮交付一次、queue follow-up 轮零正文 | `skill_set_mid_run_delivers_once_before_queue_followup` | `crates/session/tests/skill_mid_run.rs` |
| 多轮 run 的 run-end 清除 + run 2 零正文/零 tail | `preset_skill_tail_cleared_after_tool_call_run`（更新断言） | `crates/session/tests/skill_tail_cleared_after_run_end.rs` |
| 压缩估算：未交付正文计入预算（F1） | `estimated_tokens_counts_transient_skill_body` | `crates/session/src/compaction/tests.rs` |

- 全量回归：`cargo test --workspace` → 248 个测试目标全部 `0 failed`
- clippy：`cargo clippy -p opencoder-session --all-targets` → 零警告
