Commit: (working-tree, pre-initial-commit)

# 新增内置 skill：repo-local-dreaming（记忆做梦整理）+ 补注册 say-and-replay

## 背景
- 用户需要 `repo-local-dreaming`：参考 `repo-local-memory` 的周期性「做梦」整理契约——根据现状（代码基线）与时间线回顾仓库记忆，**除 changelog 外**整合冗余信息、保留当下状态快照，并内置结构硬约束（单 md ≤400 行、目录 ≤10 子文件、层级 ≤10 子目录、超限递归派生新层级）。
- `say-and-replay` 资产（`crates/core/assets/skills/say-and-replay/SKILL.md`，63 行）自 2026-08-12 起即为孤儿：历史 changelog 声称已注册进 `BUILTIN_SKILLS`，但 git 历史中注册从未落地（`git log -S 'say-and-replay' -- crates/core/src/skill.rs` 为空）。本次补注册。

## 变更
### 拆分 skill 模块（腾出注册空间）
- **`crates/core/src/skill.rs`**（798 → 590 行，≤800 ✓）：保留读侧——`Skill`/`skills_dir`/`discover(_in)`/`parse_skill`/`extract_skill_tokens`/`strip_resolved_skill_tokens`/`body_with_source` 及对应 inline tests；顶部 `mod seed;` + `pub use seed::{...}`，pub API 面与 lib.rs re-export 零变化（外部调用方 `crates/cli/src/install_tools.rs`、`crates/session/tests/*` 无需改动）。
- **`crates/core/src/skill/seed.rs`**（新增，242 行，≤400 ✓）：迁入写侧——`BUILTIN_SKILLS`/`DEP_GATED_SKILLS`/`DEPS_SENTINEL`/`seed_builtin_skills(_in)`/`seed_dep_gated_skills(_in)`/`write_install_script(_in)`/`INSTALL_SCRIPT` 及两个 install-script inline tests；`include_str!` 路径相应调整（`../../assets/...`、`../../../../scripts/...`）。

### 新增 repo-local-dreaming skill
- **`crates/core/assets/skills/repo-local-dreaming/SKILL.md`**（新增，74 行）：frontmatter 英文 description + 简体中文正文。核心章节：角色（与 `repo-local-memory` 分工：repair-on-touch 最小更新 vs 低频全量整理；**绝不改动 changelog**）、做梦四步（盘点漂移检测 → 整合去冗余 → 快照固化刷新 `Commit:` 基线 → 结构守护）、结构硬约束表（400 行/10 文件/10 子目录，超限递归拆分派生）、DREAM 块固定输出格式、与其它 skill 衔接。
- 注册进 `BUILTIN_SKILLS`（`skill/seed.rs`），首启 seed 到 `~/.opencoder/skills`，per-file 增量 never-clobber。

### 补注册 say-and-replay
- **`crates/core/src/skill/seed.rs`**：`BUILTIN_SKILLS` 于 `repo-local-memory` 后依次插入 `repo-local-dreaming`、`say-and-replay`；doc comment 补充两者定位（正交工具 `summary`/`say-and-replay`；记忆对 `repo-local-memory`/`repo-local-dreaming`）。

### 契约测试与文档
- **`crates/core/tests/skill_contract.rs`**：`seed_in_writes_all_packs_on_fresh_dir` 期望数组加 `"repo-local-dreaming"`、`"say-and-replay"`。
- **`agents/core/index.md`**（repair-on-touch）：`Skill` 段内置 skill 枚举补两 skill 定位 + 如实反映 `src/skill/seed.rs` 拆分。
- **`features/index.md`**（repair-on-touch）：「Skill 选择（TUI `$`）」条目内置清单补两 skill。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| fresh dir seed 全部 9 个内置包（含两个新 skill） | `seed_in_writes_all_packs_on_fresh_dir` | crates/core/tests/skill_contract.rs |
| seed 不覆盖用户已有文件 | `seed_builtin_skills_does_not_clobber_existing_files` | crates/core/tests/skill_contract.rs |
| 部分安装目录补齐缺失 skill | `seed_in_adds_missing_skills_to_partial_dir` | crates/core/tests/skill_contract.rs |
| install script 落盘 + 幂等（随迁移） | `write_install_script_creates_file` / `write_install_script_idempotent` | crates/core/src/skill/seed.rs |
| 拆分后 pub API 不变（全 workspace 回归） | `cargo test --workspace`（cli/session/tui 全部既有套件） | workspace |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 编译干净
- 行数：`skill.rs` 590 ≤ 800；`skill/seed.rs` 242 ≤ 400（新增）；`repo-local-dreaming/SKILL.md` 74 ≤ 400

## Impact Surface
- 用户：`$` 技能选择器新增 `repo-local-dreaming`（$repo-local-dreaming）与 `say-and-replay`（$say-and-replay）；老安装下次启动自动补 seed 两个缺失目录（never-clobber，不动用户已改文件）。
- 不影响：skill 发现/解析/token 剥离逻辑、pub API、CLI/TUI/session/store 行为边界；changelog 既有条目零改动。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [features/index.md](../../index.md)（Skill 选择条目）
- [say-and-replay 原 changelog](../2026-08-12/say-and-replay-skill.md)
