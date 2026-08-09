Commit: (working-tree, pre-initial-commit)

# feat(core): chrome-headless 技能以 bash 驱动本地 Chrome（dep-gated opt-in）

## 背景

2026-08-07（见 `2026-08-07/remove-tool-agent-browser-capabilities.md`）移除了浏览器 agent 与
bundled browser 能力，`DEP_GATED_SKILLS` 一度仅剩 `ssh-pty`。但「抓取网页内容」的需求仍在：
做调研、读 JS 重渲染页面、看 SERP 结果。本变更以一种**不打包任何浏览器二进制**的方式重新引入
该能力——`chrome-headless` 技能指导 agent 用既有 **bash 工具**驱动用户本地已安装的
Chrome/Chromium（`--headless=new --dump-dom`），结构化数据优先走公开 API（curl 直连
GitHub/HuggingFace JSON API），仅当无 API 或页面 JS 渲染时才回退 headless DOM 渲染；并支持
`--proxy-server` 隧道。

由于依赖一个本地 Chrome（不随二进制分发），该技能放入 `DEP_GATED_SKILLS` 桶：用户运行
`install-skills-dep.sh` 创建 `.skills-deps` sentinel 后首启才 seed，fresh install 默认不出现。

## 变更

- **`crates/core/src/skill.rs`**：`DEP_GATED_SKILLS` 新增 `chrome-headless` 项（`include_str!`
  内嵌其 `SKILL.md`）；同步更新该常量 doc-comment，说明两技能各自依赖（ssh-pty 需 tmux；
  chrome-headless 需本地 Chrome，运行时探测、不打包）。
- **`crates/core/assets/skills/chrome-headless/SKILL.md`**（新文件，98 行）：技能指令包。frontmatter
  声明 `name`/`description`；正文给出 bash 驱动原语——探测 Chrome（`command -v`）、
  `--dump-dom` 落盘后用 read 工具检视、GitHub/HuggingFace curl API-first、
  `--proxy-server`/`--proxy-bypass-list` 隧道、以及「无 Chrome 时停下询问用户、不无人值守装包」
  的安全约束。
- **`crates/core/tests/skill_contract.rs`**：扩展既有 `dep_gated_skills_do_not_clobber_existing`
  测试，预置一个用户改写的 `chrome-headless/SKILL.md`，断言 seed 后该文件被**原样保留**（never-clobber）。
- **范围**：纯增量。一个 `include_str!` 资产 + 一个 const 数组条目 + 一处既有测试的断言扩展；
  未改 seeding 路径逻辑、未改 trait/配置/数据形状。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| dep-gated 技能仅在 sentinel 存在时 seed | `seed_dep_gated_skills_only_when_sentinel` | `crates/core/tests/skill_contract.rs` |
| dep-gated 技能 never-clobber（含 chrome-headless 用户文件原样保留） | `dep_gated_skills_do_not_clobber_existing` | `crates/core/tests/skill_contract.rs` |

- 全量回归：`cargo test --workspace` → **2214 passed / 0 failed**
- 隔离回归（opencoder-core）：`cargo test -p opencoder-core --test skill_contract` → **12 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：`skill.rs` 776（≤ 800）；`skill_contract.rs` 212（≤ 800）；`SKILL.md` 98（≤ 400，新增）
