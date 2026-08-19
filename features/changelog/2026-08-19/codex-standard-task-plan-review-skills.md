Commit: (working-tree, post-3320cbb)

# 内置 task-plan / review 对齐 Codex skill 标准

## Context

内置 `task-plan` 仍围绕固定 STATUS 块和仓库特定 gate 展开，缺少 Codex 当前 skill 对证据成熟度、跨服务合约、持续保鲜、生产等价验收和遗漏复查的完整上线闭环口径。内置 `review` 又被压缩成五问摘要，无法充分表达完成百分比的证据依据、全局影响、交付 blocker 和持续可承载判断。

本轮以本机 Codex skill 为基准：`task-plan` 直接对齐 Codex 当前版本；`review` 参考 Codex `say-and-replay` 的完成度、细粒度证据、卡点和闭环路径模型，并保留 review 独有的 go-live 裁决责任。

## Change Summary

- `crates/core/assets/skills/task-plan/`：内置正文更新为 Codex 上线闭环规划协议，覆盖问题与事件流重建、五级证据成熟度、合约/保鲜矩阵、根因与缺口地图、生产等价验证、遗漏复查和发布关键路径。
- `task-plan/references/`：随包增加 `launch-closure-plan-checklist.md` 与可选 `any-home-plan-run.md`，按 progressive disclosure 承载细节，避免主 skill 继续膨胀。
- `crates/core/assets/skills/review/SKILL.md`：由固定五问升级为证据驱动评审；强制给出保守完成百分比，区分验证方法与证据，检查影响面、发布责任、已解除/当前卡点，并裁决 `go-live ready` 或 `not ready`。
- `crates/core/src/skill/seed.rs`：抽取统一 `seed_skill_packs` 写入路径，支持 `references/*` 等嵌套 bundled resources；仍逐文件增量写入，任何已有用户文件均不覆盖。
- `do-and-done` / `summary` / `submit`：直接消费者同步从已退役的固定 STATUS 块切到闭环计划、当次证据与 review 上线结论，避免内置工作流等待新 task-plan 不再产出的字段。
- `crates/core/tests/skill_contract.rs`：契约断言改为新 task-plan / review 语义；新增嵌套 references 首次 seed、never-clobber 和下游不再依赖 STATUS 块的覆盖。

## Impact Surface

- 新安装会直接获得新版两个 skill 及 task-plan references。
- 已安装用户的 `SKILL.md` 继续遵守 never-clobber，不会被升级覆盖；缺失的 references 会在下次 seed 时补齐，用户已自定义的同名 reference 也会保留。
- `say-and-replay` 本身未改：它继续负责检查点进度对齐；`review` 负责交付质量、全局影响和上线结论。

## Validation

- skill-creator `quick_validate.py`：`task-plan`、`review` 均 `Skill is valid!`。
- `cargo test -p opencoder-core`：254 passed / 0 failed，其中 `skill_contract` 19 passed。
- `cargo clippy -p opencoder-core --all-targets -- -D warnings`：零警告。
- `cargo build -p opencoder-core`：成功。
- `cargo fmt --all --check`、`git diff --check`：通过。

## Related Docs

- [core 模块](../../../agents/core/index.md)
- [能力地图](../../index.md)
