Commit: (working-tree, post-1ba8f426)

# review / say-and-replay 五问复述契约

## Context

内置 skill `review`（REVIEW 块）与 `say-and-replay`（REPLAY 块）此前对「五问」覆盖不齐：review 只有 `goal_met` 判定不复述目标、无完成点清单/卡点/稳定 TODO；say-and-replay 的 `done` 未强制「怎么验证的」（方式），`blocked` 只覆盖当前卡点、不含过程中已解除的。本轮增强两者产出契约：**五问必答**——① 复述需求目标 ② 做了哪些事情 ③ 遇到了什么卡点（含已解除 + 解除方式）④ 每个完成点怎么验证的、证据是什么 ⑤ 下一步 TODO。

## Change Summary

- `crates/core/assets/skills/say-and-replay/SKILL.md`（63→67 行，增量强化）：
  - REPLAY 块 `done:` 条目新增 `verify:`（怎么验证的：命令/方法），与既有 `evidence:`（当次证据）并列——**缺一不计入 completed**，归入 `doing`。
  - 卡点拆两段：新增 `encountered:`（全程遇到的卡点，含已解除——`status: resolved` + `resolution:` 解除方式；无则 none）+ 保留 `blocked:`（当前卡点）。
  - 「角色」与「原则」显式声明「五问必答」（任一缺失或空泛即 REPLAY 不完整）；frontmatter `description` 同步补 verify-method + encountered blockers。
- `crates/core/assets/skills/review/SKILL.md`（147→162 行，gate 机制原样保留）：
  - 「角色」四问扩展为「先答五问、再评 gate」。
  - REVIEW 块顶部、`goal_met` 之前新增固定字段：`goal:`（复述原始需求目标+验收标准，对照 STATUS goal 标注偏航）、`done:`（逐条完成点 `| verify: | evidence:`）、`blockers:`（含 resolved + 解除方式；无则 none）、`next_todos:`（ready→后续建议或 none；not ready→与 `gaps:` 对应的修复 TODO）。
  - 「结论规则」新增：五问任一缺失或空泛（无证据）→ 视同证据不充分，`verdict: not ready`。
  - frontmatter `description` 同步（restates goal / replays done+evidence / blockers / next TODOs）。
- **seeding 语义不变**：资产经 `include_str!` 编译期内嵌、首启 per-file seed 且 never-clobber——老用户已存在的 `~/.opencoder/skills/{review,say-and-replay}/SKILL.md` **不会被覆盖**（用户改动永久存活）；删除对应文件后重启即可拿到新版。
- 记忆文档 repair-on-touch：`features/index.md`、`agents/core/index.md` 内置 skill 清单各补一句五问复述职责。

## Validation（当次实跑）

- `cargo test --workspace`：172 个套件全绿，合计 **2804 passed / 0 failed**（回归基线 2802 + 本轮新增 2，净增无删除）；其中 `skill_contract` **15 passed**（基线 13 + 新增 2），`-p opencoder-core` 口径 238 passed / 0 failed（unit 134 + 集成 104）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（Finished dev profile，3m30s）。
- `cargo build --workspace`：编译干净（Finished dev profile，56s）。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `skill_contract.rs::seeded_review_skill_requires_five_question_recap` | integration | seed 后 review SKILL.md 含 `goal:`/`done:`/`verify:`/`blockers:`/`next_todos:` + frontmatter name/description——asset 字段被改丢即红 |
| `skill_contract.rs::seeded_say_and_replay_skill_requires_five_question_recap` | integration | seed 后 say-and-replay SKILL.md 含 `goal:`/`verify:`/`encountered:`/`blocked:`/`remaining:` + frontmatter——同上防回归 |

纯 markdown asset + 追加测试，无 Rust 逻辑改动；无删测试 / 无 `#[ignore]` / 无弱断言。
