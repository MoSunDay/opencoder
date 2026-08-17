Commit: (working-tree, post-5ac8cc9)

# review / say-and-replay 输出契约补完成度百分比（progress%）

## Context

用户四段结构要求「核心 TODO 清单 + 完成度」带显式百分比。此前现状：`review` 的 REVIEW 块有 `done:` 清单但**无 progress 字段**（仅定性 `goal_met`）；`say-and-replay` 的 REPLAY 块有 `progress: X/Y` 比值但**无百分比**。`task-plan` 的 STATUS 块已采用 `progress: <0-100>%`（`floor(completed/total × 100)`，仅 verify+evidence 俱全计入 completed）约定——本轮把两个 skill 的固定输出对齐到同源口径，严格覆盖四段结构：① 复述需求目标（goal）② 核心 TODO 清单 + 完成度 completed/total + 百分比（progress + done）③ 每条完成 TODO 的证据（verify + evidence）④ 下一步 TODO（next_todos / remaining）。

## Change Summary

- `crates/core/assets/skills/review/SKILL.md`（162→163 行，增量强化）：
  - 五问第 2 问扩展：「做了哪些事情？做到了多少？（逐条完成点回放 + 核心 TODO 完成度：completed/total + 百分比，向下取整）」。
  - REVIEW 固定块 `goal:` 之后新增 `progress: <completed 数>/<总数>（<0-100>%，向下取整）`，行注释钉住口径：仅计入 verify+evidence 俱全的 completed，同 task-plan progress%。
  - 「结论规则」字段缺失清单补入 `progress`（缺字段即视同证据不充分 → not ready）。
  - frontmatter `description` 补 `quantifies progress (completed/total + percent)`。
- `crates/core/assets/skills/say-and-replay/SKILL.md`（67 行不变）：
  - REPLAY 块 `progress:` 由纯比值追加为 `<completed 数>/<总数>（<0-100>%，向下取整）`。
  - 「角色」五问② 与「字段语义」progress 条目补百分比口径（`floor(completed 数/总数 × 100)`）。
  - frontmatter `description` 同步补 quantifies progress。
- 记忆文档 repair-on-touch：`agents/core/index.md` 内置 skill 清单中 `review` / `say-and-replay` 的五问职责描述各补「完成度 completed/total + %」措辞（`features/index.md` 的五问枚举句仍为真，未动）。
- **已知限制（本轮不改）**：seed 的 never-clobber 策略不覆盖已存在的 `~/.opencoder/skills/<name>/SKILL.md`，老安装需删除对应目录后重启才拿到新文案；该策略属独立设计决策。
- **不动项**：`seed.rs` never-clobber 逻辑、其它 skill 资产、autopilot 激活路径均未改。

## Validation（当次实跑）

- `cargo test --workspace`：173 个套件全绿，合计 **2820 passed / 0 failed**（上一轮提交基线 2804；工作区含并行 autopilot 迭代的未提交新增测试，净增无删除）。其中 `skill_contract` **16 passed**（HEAD 15 + 并行迭代新增 task-plan 测试 1；本轮为两个既有五问契约测试的断言增强，未新增测试函数）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（Finished dev profile）。
- `cargo build --workspace`：编译干净（Finished dev profile，2.37s）。
- flaky 备注：首次全量运行中 `session::steer_reabsorb::late_steer_reabsorbed_after_run_loop_returns` 在并行编译负载下失败一次（断言 run_loop 恰两遍，得 1），隔离复跑 **3/3 通过**，二次全量绿；该区域属并行迭代改动面（`runner/steer.rs`），非本轮触及。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `skill_contract.rs::seeded_review_skill_requires_five_question_recap` | integration | 字段列表加 `progress:`；并精确断言 REVIEW 块输出含 `progress: <completed 数>/<总数>（<0-100>%`——字段被改丢或百分比被删即红 |
| `skill_contract.rs::seeded_say_and_replay_skill_requires_five_question_recap` | integration | 字段列表加 `progress:`；断言含 `（<0-100>%` 与 `百分比`（字段语义解释）——防回退为纯比值 |

纯 markdown asset 强化 + 既有测试断言增强，无新 Rust 逻辑；无删测试 / 无 `#[ignore]` / 无弱断言。
