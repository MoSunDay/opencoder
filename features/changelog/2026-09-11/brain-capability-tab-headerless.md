Commit: working-tree

# 能力库页签直连能力 CRUD 表，大脑页去掉 PageShell 页头

## Context

大脑调度工作台的「能力库」页签此前是两层结构：外层 `/api/brain/library` 聚合表（`expandable` JSON 展开行、`成熟度` 列、`标记稳定/改为草稿` 按钮），内层 Collapse「维护能力描述与目标绑定」再包一层 `brainPanel.jsx` 的能力 CRUD 表。产品口径上能力库没有"稳定/草稿"之分，成熟度只是服务端证据字段，Web 不应提供该交互；两层表格也让用户先看到一份不可编辑的目录，再展开才能改数据。同时 brain 页的 PageShell 页头（标题「大脑调度」+ 描述）与页内 Tabs 标题重复。

## Change Summary

「能力库」页签的 children 直接换成 `<BrainPanel onNotice={onNotice} />`：行点击进「编辑能力」抽屉、「新建能力」按钮、按意图语义搜索与删除全部保留，聚合表与 Collapse 一并移除（`apiPost` 的 `/api/brain/library/:id/stable` 调用点随之消失，该文件只再依赖 `apiGet`）。`reload()` 仍拉 `/api/brain/library` 并保留 `capabilities` state —— 计划库编辑器 `editor.jsx` 的 StepEditor 用它渲染目标下拉。

`nav.js` 删除 `PAGE_META.brain`，新增 `export const HEADERLESS_PAGES = ['brain']` 作为"页面自带标题、不配页头文案"的显式清单；PageShell 对没有 meta 的 page 自然只渲染无页头的 `.oc-page` body，组件本身零改动。

## Impact Surface

- `crates/web/spa/src/brain/workbench/index.jsx`（能力库页签 + import 收敛）
- `crates/web/spa/src/nav.js`（`HEADERLESS_PAGES` 新增、`PAGE_META.brain` 删除）
- `crates/web/spa/src/nav.test.js`、`crates/web/spa/src/shell/pageShell.dom.test.jsx`（契约断言）
- `crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx`（新增）
- 服务端零改动：`/api/brain/library`（GET）仍供计划编辑器使用，`POST /:id/stable` 只是失去 Web 调用方

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 页面键覆盖 = PAGE_META + headerless 清单 | `covers every page key exactly` | `crates/web/spa/src/nav.test.js` |
| headerless 页不进 PAGE_META 且仍属 IA | `keeps headerless pages out of PAGE_META while they stay in the IA` | `crates/web/spa/src/nav.test.js` |
| brain 页无页头、body 照常渲染 | `renders the headerless brain page body without any page header` | `crates/web/spa/src/shell/pageShell.dom.test.jsx` |
| 工作台无「大脑调度」标题与描述 | `renders the brain page without a PageShell header` | `crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx` |
| 页签即 CRUD 表：无 Collapse/成熟度/稳定草稿按钮/展开行，且不写 library | `shows the capability CRUD table directly with no maturity rows or library mutations` | `crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx` |
| 「新建能力」对话框 | `opens the 新建能力 dialog from the capability tab` | `crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx` |
| 行点击进「编辑能力」抽屉 | `opens the 编辑能力 drawer when a capability row is clicked` | `crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx` |

- 定向回归：`npx vitest run src/nav.test.js src/shell/pageShell.dom.test.jsx src/brain/workbench/tests/capabilities.dom.test.jsx src/brainPanel.dom.test.jsx` → 4 files / 40 passed / 0 failed（nav 21 + pageShell 6 + capabilities 4 + brainPanel 9）
- 相关面回归：`npx vitest run src/nav.test.js src/shell src/brain src/brainPanel.dom.test.jsx src/app.dom.test.jsx src/fleet` → 15 files / 112 passed / 0 failed
- 全量 SPA 回归（本改动单独在树、他人在制品尚未铺回时）：`npx vitest run` → 83 files / 675 passed / 0 failed
- 全量 SPA 回归（他人在制品铺回后的合并态）：`npx vitest run` → 88 files / 685 passed / 4 failed；4 个失败全在他人文件且与本改动无引用关系——`src/ui/executionEvents/logs.dom.test.jsx`（3 例，import `useExecutionEvents.js` / `executionLogs.jsx`）与 `src/brain/workbench/tests/draft.test.js`（1 例，import `editor/model.js` / `editor/draft.js`，断言 flow 返回边 `复测发现问题，回到修复`），两者均未 import `nav.js` 或 `workbench/index.jsx`
- `brainPanel.dom.test.jsx`（9 例）、`app.dom.test.jsx`（21 例）、`fleet/`、`shell/pageShell.dom.test.jsx` 既有断言一律未删未放宽
- `app.dom.test.jsx` 的「点击 Agent 分类落到 brain 页并渲染出『工作台』」在去页头后仍通过
- `dist` 已用 `scripts/build-spa.sh` 重建（本次改动的源码已进入内嵌产物；他人在制品铺回后 `dist` 会再次相对 `src` 漂移，最终重建由最后落地方统一执行）：`crates/web/spa/dist/static/app.js` 对 `维护能力描述与目标绑定`、`成熟度`、`标记稳定`、`改为草稿`、`维护能力与版本化计划` 全部 0 命中，`新建能力` 保留 3 处
- `scripts/check-spa-drift.sh` → `spa dist: no drift (build 2/3)`，exit 0（首轮因压缩器标识符命名不稳定报 only-app.js 差异，脚本按既有约定重建重试后收敛）
- `cargo build --workspace` → Finished dev profile，exit 0；dist 是编译期内嵌（`crates/web/src/html.rs:23-26`），构建通过即证明新产物可被 `include_bytes!` 嵌入
- `cargo test --workspace` / `cargo clippy --workspace` 未跑完：`opencoder-brain` 处于他人 flow 改动的半成品状态（`mod flow;` 已声明、`crates/brain/src/{execution,ontology}/flow.rs` 仍只在 `stash@{0}^3`）报 `E0583` / `E0062`；破损发生在本次 `cargo build --workspace` 成功之后，本任务 diff 不含任何 `.rs`，未代为修复。规则 02 的全量 Rust 回归待 flow 收敛后补跑
- 服务端（control/worker/brain/store）零改动

## 工作树并发状态

多 agent 共用一个工作树，发布 agent 会 `git stash push -u` 暂存全部未提交在制品（本次为 `stash@{0}: preserve-concurrent-work-before-web-release-202609111752`）并用回滚后的源码重建 `dist`，本改动因此被整体回滚过一次。恢复手段：tracked 用 `git restore --source=stash@{N} --worktree -- <path>`，untracked 用 `git show 'stash@{N}^3:<path>'`；若 stash 差异混有他人 hunk（`nav.js` / `nav.test.js` 即是），只手工施加本任务 hunk，不要整文件恢复。不要 `git stash pop` / `drop`，他人在制品须原样留在 stash。上述回归数据均为重放后实测。

稳定文档已同步：[Web 模块](../../../agents/web/index.md)、[大脑调度功能](../../brain/index.md)。
