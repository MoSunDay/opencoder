Commit: (working-tree, post-860831d)

# review 内置 skill 去模板化：五问即产出 + description 对齐用户当前版

## Context

内置 `review` skill 此前强制固定输出模板（REVIEW 块：`goal:`/`progress:`/`done:`/`verify:`/`blockers:`/`next_todos:` 字段清单，162 行）。本轮以用户本地当前版（`~/.opencoder/skills/review/SKILL.md`）为准内置化：**移除固定输出模板，答好五问本身就是产出**（"No fixed output template; answering the five questions well IS the output"），证据纪律原样保留。

## Change Summary

- `crates/core/assets/skills/review/SKILL.md`（162→31 行）：
  - REVIEW 块模板整体移除，改为五个问题章节（问一：原始需求目标 / 问二：做了哪些事情及完成度 / 问三：卡点 / 问四：逐项验证+证据 / 问五：下一步 TODO）+「上线结论」（五问答完裁决 `go-live ready | not ready`；五问任一缺失或空泛 → 直接 not ready）。
  - 证据纪律保留：完成点必须带 verify + 当次证据（缺一不计入完成）；完成度 = completed/total + 百分比（向下取整）；**没有证据 = 没有通过**，证据必须是**当次实跑**。
  - frontmatter `description` 与用户当前版逐字一致：去掉「Read-only post-completion assessment organized entirely around」前缀与「Never edits code or commits」后缀（精简为 "five mandatory questions — …"）。
- `crates/core/tests/skill_contract.rs`：`seeded_review_skill_requires_five_question_recap` 由字段断言（`goal:`/`progress:` 等）改为章节断言（问一~问五标题）+ 证据纪律断言（`completed/total`、`向下取整`、`没有证据 = 没有通过`、`当次实跑`、`go-live ready | not ready`）；移除 `Never edits code or commits` 断言（description 已不含该短语）。
- `agents/core/index.md`：内置 skill 清单中 `review` 职责描述去掉「只读」限定（repair-on-touch，与资产现状对齐）。
- **seeding 语义不变**：资产 `include_str!` 编译期内嵌、首启 per-file seed 且 never-clobber——已 seed 过的机器上现存 `~/.opencoder/skills/review/SKILL.md` **不会被覆盖**；删除该文件后重启二进制即可拿到新版。

## Validation（当次实跑）

- `cargo test -p opencoder-core --test skill_contract`：**16 passed / 0 failed**。
- `cargo test --workspace`：全绿（合计见下）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `skill_contract::seeded_review_skill_requires_five_question_recap` | integration | seed 后 review 资产含问一~问五五章节 + completed/total + 向下取整 + 没有证据=没有通过 + 当次实跑 + go-live verdict |
| `skill_contract::seed_in_writes_all_packs_on_fresh_dir` | integration | review 仍在内置清单（fresh dir 全量 seed） |
| `skill_contract::seed_builtin_skills_does_not_clobber_existing_files` | integration | 用户改动不被内置 seed 覆盖 |
