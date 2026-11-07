# 文档链接终清：docs/ 树移除后的全部引用归位

一句话：`docs/` 目录移除后残留的 27 个文件内引用全部归位——README
中英文版的 Windows / Agent 平台链接改指 `features/platform/**` 能力页、
快速上手引言与目录树中的 `docs/` 行删除，agents/features 索引与
rules 06/07 中指向 `docs/*.md` 的链接按语义改指真实事实源
（`crates/**` 模块 / `scripts/**` 脚本与验收），各索引 `Commit:`
锚点同步到 `5afb497be`。

## 变更清单

- `README.md` / `README.en.md`：`docs/windows.md` →
  `features/platform/windows/index.md`、`docs/agent-platform.md` →
  `features/platform/agent-platform/index.md`；快速上手引言（对应
  quickstart 文档已删）与目录树 `docs/` 行移除。
- `agents/**/index.md`（14 个）与 `features/**/index.md`（10 个）：
  `Commit:` 锚点统一到 `5afb497be`；docs 链接按语义重指——
  运行协议 → `crates/control/src/api/brain_runs`、动态步骤 →
  `crates/dag/src/dynamic.rs`、步骤控制 → `crates/dag/src/control/mod.rs`、
  平滑发布 → `scripts/acceptance/smooth_release/main.py`、rootfs 制备 →
  `scripts/prepare-dag-rootfs.sh`、Codex 启动脚本 →
  `crates/dag-runtime/src/sandbox/codex/mod.rs`、外部输入协议 →
  `crates/session/src/harness/codex/control/mod.rs`、项目 CLI/文件/概述 →
  `crates/ctl/src/cmd/project.rs` / `crates/control/src/project_files` /
  `crates/web/src/api_project/summary.rs`、维护窗口 →
  `scripts/acceptance/maintenance_release/main.py`。
- `repo-memory.md`：锚点更新；`rules/06`、`rules/07`：`../docs/*`
  改指 `../crates/**` 真实模块。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 变更 md 相对链接有效性 | 27 个变更文件全部相对链接逐一解析 | — |

## 定向回归

- 相对链接检查 → 0 失效。
- 纯文档变更，无代码/测试影响面（规则 02：文档只查内容与链接）。

## 配置与依赖

- 无。

## 回滚步骤

- 还原对应文档文件即回滚；无数据迁移。

## 遗留

- `Commit:` 锚点为信息性字段、无脚本校验，随下次代码变更再同步。
