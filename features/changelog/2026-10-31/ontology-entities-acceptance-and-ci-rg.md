# Ontology 实体页验收脚本对齐目录标签布局 + brain-e2e 安装 ripgrep

一句话：实体页自目录多对多改造（d8cdb050d）后内容区改为「实体|目录」标签、移除「全部实体」切换与目录树点击流，三个验收脚本同步删除过期交互；brain-e2e 环境补装 ripgrep，供节点内 notepad 搜索工具（外部 `rg` 命令）在 CI 全链路中可用。

## 变更清单

- `scripts/acceptance/ontology/entities.mjs`：删除「全部实体」按钮交互、目录树点击/目录指派流与对应 `directory-empty` 度量——默认标签即全部实体列表，无目录实体不再需要先切「全部实体」。
- `scripts/acceptance/ontology/catalog.mjs`：删除打开能力抽屉前的目录树点击步骤。
- `scripts/acceptance/spa_responsive_ontology.js`：删除 `directories/tree` 固定响应（页面不再请求该接口）。
- `.github/workflows/brain-e2e.yml`：apt 依赖加 `ripgrep`（`crates/tui/src/notepad/search.rs:106` 以外部 `rg` 命令实现搜索）。

## 测试覆盖（功能 → 测试名）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 实体页目录标签布局交互流（脚本对齐对象） | `checkEntities`（无目录实体可见、搜索过滤、重试路径） | `scripts/acceptance/ontology/entities.mjs` |
| 能力目录抽屉流程 | `checkCatalog` | `scripts/acceptance/ontology/catalog.mjs` |
| 响应式断点渲染（fixture 面收窄） | `spa_responsive_ontology.js` | `scripts/acceptance/` |

## 定向回归

- `node --check` 三个脚本 → 语法通过。
- Playwright 浏览器验收需要 Server+SPA 运行栈，本轮未在本地复跑，由 brain-e2e CI 首跑验证（见遗留）。

## 配置与依赖

- CI apt 新增 `ripgrep`；无 Rust/SPA 依赖变化，无运行时配置变化。

## 回滚步骤

- 还原四个文件即回滚；无数据迁移、无协议变更。

## 遗留

- 浏览器验收链路依赖 brain-e2e CI 首跑确认（本地无运行栈）；若实体页交互流与脚本假设不符，以 CI 失败为准回滚脚本改动。
