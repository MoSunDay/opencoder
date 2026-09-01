Commit: b243c2b (latent 深路径 fixture 根深抬至 520——Source 线独占 500 字符窗)

# latent 深路径 fixture 修复：哨兵恢复甄别力

## 背景

`4337e4f`（task-plan SKILL.md 瘦身）落库后，哨兵测试
`tools::latent::tests::long_source_path_keeps_question_within_unlock_window`
在 HEAD 确定性红（`latent.rs:411` panic，隔离复跑 0.03s 即失败，非负载竞态）。
机制：fixture 根深 240 时 Source 行 262 字节，瘦身后 seed 体首个 `question`
提及（「澄清协议（question 工具）」节）落在注入体 ~270 字符处，进入 500 字符
前缀窗，fixture 前置断言 `!prefix.contains("question")` 失败。

探针已证实**生产语义完好**：深 HOME 下 unlock 仍正常 fire（`unlocked
contains question = true`，Source 线位置无关路径生效）。红的只是 fixture
前置条件——但该哨兵正是为「seed 内容漂移即死」设计，若只改断言不改 fixture
深度，测试将对「窗口式扫描回归」永久失明。

## 实现（`crates/session/src/tools/latent.rs` 仅测试模块，9+/6-）

- `deep_skills_root(240)` → `deep_skills_root(520)`：根深抬至 520 字节后
  `> Source: <root>/skills/task-plan/SKILL.md` 一行单独即超 500 字符窗，
  `question` 依构造位于窗外——**断言零改动**，甄别力恢复（seed 体仍带
  `question` 载荷，SKILL.md:10/15/16，窗口式扫描若回归即死）。
- seed/discover 收敛到 `root.join("skills")` 子目录（两个深路径测试一致），
  与生产 `~/.opencoder/skills` 布局对齐。

## 回归

- `cargo test -p opencoder-session --lib latent::` → 27 passed / 0 failed
  （当次复跑，含修复后哨兵与对照测试
  `long_source_path_review_body_still_unlocks_nothing`）
- 全仓门禁见文末补记。

## Related Docs

- [agents/session](../../../agents/session/index.md)
- [skill-body 注入与 latent 解锁](../../index.md)
