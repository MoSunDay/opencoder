# Web Ontology 实体页去掉目录栏，改为全量平铺列表

一句话：实体 tab 移除左侧「目录」栏（目录树、目录筛选、目录管理与“全部实体”切换），进入即显示全部实体（含未归入目录的实体）；搜索、实体类型筛选、状态筛选和分页直接作用于全量列表。目录的创建、挂载与调整不再经由 Web，统一走 Ontology CLI（`directories` / `relationships`，见 rules/10-ontology-cli.md），服务端目录 API 与数据不变。

## 变更清单

- `crates/web/spa/src/ontology/pages/EntitiesPage.tsx`：删除目录 Section、目录树/移动端目录选择器、`selectedDirectory`/`showAll`/`expandedDirectories` 状态与目录成员计算；资源加载去掉 `api.directories`；布局对齐实体类型/关系类型页（`admin-content` + DataTable）。
- `crates/web/spa/src/ontology/pages/entities/CreateEntityModal.tsx`：移除 `directoryId` 入参与 payload 中的 `directory_id`。
- 删除 `crates/web/spa/src/ontology/pages/DirectoryActions.tsx`、`entities/directories.ts` 及相邻测试（目录树/成员/选项纯函数不再被生产代码引用）。
- 冗余清理：`crates/web/spa/src/ontology/api.ts` 移除无人调用的 `directories/createDirectory/moveDirectory/updateDirectory` 客户端与 `DirectoryItem` 类型（服务端目录 API 不变，由 CLI 使用）；`scripts/acceptance/spa_responsive_ontology.js` 移除不再被请求的 `directories/tree` fixture。
- 验收脚本同步：`scripts/acceptance/ontology/entities.mjs` 去掉目录树/目录选择步骤（实体按钮进入即见），`catalog.mjs` 直接搜索定位能力实体，不再点击目录树节点。
- 文档：`features/ontology/index.md`、`agents/web/ontology.md` 更新实体页行为描述，删除目录栏相关段落。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 实体页首载即显示全部实体（含未归入目录）、目录选择器不存在、搜索收敛/恢复 | `shows all entities on first load and applies search` | `crates/web/spa/src/ontology/pages/entities/EntitiesPage.test.tsx` |
| 实体加载失败提示与重试恢复列表 | `shows load errors and lets retry restore the list` | `crates/web/spa/src/ontology/pages/entities/EntitiesPage.test.tsx` |
| 实体创建 payload 不再携带 `directory_id` 且重试复用请求 ID | `loads required fields, distinguishes NFS paths and retains the request ID across a retry` | `crates/web/spa/src/ontology/pages/entities/CreateEntityModal.test.tsx` |
| 首页导航仍按 Server 默认数据空间请求实体接口（页面回归） | `reads and creates in the Server namespace despite a stale browser environment selection` | `crates/web/spa/src/ontology/navigation/serverEnvironment.test.tsx` |

- 定向回归（`crates/web/spa` 下）：`npx vitest run src/ontology/pages/entities` → 3 文件 / 4 项通过；`npx vitest run src/ontology/navigation/serverEnvironment.test.tsx` → 1 文件 / 3 项通过。
- 类型检查：`npm run typecheck:ontology` → 通过，零错误（含 api/types 清理后复跑）。
- 未修改 Rust，Clippy 不适用；未运行全量测试。Ontology 浏览器验收（`scripts/acceptance/ontology/`）需独立 Server 与 rootfs，脚本已同步但本轮未执行。
