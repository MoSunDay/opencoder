---
name: submit
description: Stage only meaningful files for commit (explicitly exclude cache/build/temp/IDE junk), generate a changelog entry following the repo convention, then create a conventional-commit. Runs the full regression gate first. Does not push, amend, or force.
---

# submit —— 暂存 + changelog + 提交契约

## 角色
提交契约。在 `review` 给出 `go-live ready` 之后介入，把已验证的变更**干净地**提交：只暂存有意义的文件、生成 changelog、按仓库风格 commit。

> **不可逆边界**：本 skill 只做 `git add`（精准）+ `git commit`。**不 push、不 amend、不 force、不动 remote**。需要 push 时单独停下上报人工。

## 前置 gate（先验证再提交）
提交前必须全绿，否则停下上报缺口（呼应 `rules/02-regression-gate.md`）：
- `cargo test --workspace` → 全绿（含适用集成 / e2e）。
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- `cargo build --workspace` → 编译干净。

任一不过 → **不提交**，回 `do-and-done` 修复。

## 暂存策略（只暂存有意义的文件）

### 1. 审查工作区
```
git status
git diff          # 未暂存改动
git diff --cached # 已暂存改动（若有）
git log --oneline -5  # 仓库风格参考
```

### 2. 排除清单（绝不提交）
显式跳过以下 cache / build / 临时 / IDE / 本地态文件（即使未被 .gitignore 覆盖也要主动排除）：
```
target/  node_modules/  __pycache__/  dist/  build/  out/
.opencode/  .opencoder/  .sst  .turbo  .codex  .serena/  .omo/  .idea/  .vscode/
logs/  tmp/  playground  *.bak*  .bak-*/  Session.vim
link_repos  refs  /result  UPCOMING_CHANGELOG.md
*.lock.bak  tsconfig.tsbuildinfo  a.out  *.bun-build
.env  .env.local  （以及任何含密钥/凭证的文件）
```
注意：`Cargo.lock` / `package-lock.json` 这类**依赖锁文件应随源码提交**，不要排除。

### 3. 精准暂存（不盲目 `git add -A`）
- 优先 `git add <显式路径>` 逐个加入真实变更（源码、测试、文档、memory、changelog）。
- 若用 `git add -A`，**必须**随后用 `git status` 复核，对误入的排除清单项 `git restore --staged <path>` 撤出。
- **绝不**暂存：排除清单中的任何项、含密钥/凭证的文件、与本任务无关的改动。

### 4. 复核暂存集
```
git diff --cached --stat   # 一览：文件数、行数
git diff --cached          # 逐行确认无垃圾、无调试输出、无密钥
```
发现排除清单项或可疑内容 → 撤出并重审，**不带着垃圾提交**。

## changelog 生成
按仓库约定在 `features/changelog/<YYYY-MM-DD>/<topic>.md` 新建条目（日期取当天）。结构遵循本仓库既有 changelog 格式：
```
Commit: (working-tree, pre-initial-commit)

# <标题：一句话主题>

## 背景
<为什么做这个变更；用户报告的问题 / 要达成的目标>

## 变更
### <主题>
- **`<文件路径>`**：<逐项说明，标注关键 file:line>

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| ... | ... | ... |

- 全量回归：`cargo test --workspace` → <结果>
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → <结果>
- 行数：<文件> <N> ≤ <400|800>

## Impact Surface
- <对用户/调用方可感知的影响>
- <不影响：CLI/Web/session/store/... 边界>

## Related Docs
- [agents/<x>](../../agents/<x>/index.md)
- [既有相关 changelog](../<date>/<file>.md)
```
- 同时把 changelog 文件加入本次提交（与代码同 commit）。
- 触及区 memory（`agents/*`、`features/*`）按 `repo-local-memory` 做 repair-on-touch，一并暂存。

## commit message 规范（conventional + 仓库风格）
跟随仓库 `git log` 既有风格（参考最近提交）：
```
<type>(<scope>): <摘要>

[可选正文：动机 / 关键变更 / 测试结果]
```
- `type`：`feat` / `fix` / `docs` / `test` / `refactor` / `chore` / `perf`
- `scope`：受影响模块（如 `tui` / `session` / `store` / `e2e` / `changelog`）
- 摘要一行，中英文均可（与仓库历史一致）。
- 正文可选，列动机 + 关键变更要点。

示例：
```
fix(tui): Ctrl+D 退出时取消运行中 turn，加有界 worker 关闭
```

## 提交
```
git commit -m "<type>(<scope>): <摘要>"
```
- 多行 message 用 `git commit -F <file>` 或多个 `-m`。
- 提交后 `git log --oneline -1` + `git show --stat HEAD` 取证确认。

## 不可逆操作 —— 暂停协议
- **绝不**自行 `git push` / `--amend` / `--force` / 改 remote / 删分支。需要 push 时**停下上报**：列清待批操作、当前 commit hash、未决项，交还人工。
- 提交本身（`git commit`）在本 skill 范围内授权执行；push 及以上不可逆操作不在内。

## 与其它 skill 的衔接
- 仅在 `review` 给出 `go-live ready` 后执行（gate 已绿）。
- 消费 `task-plan` 的 STATUS 块汇总变更摘要，作为 changelog/commit message 的素材来源。
- 提交完成 → 输出 commit hash + 变更摘要 + （如需 push）待批上报。
