Commit: 17ca79c3b55e06c4a2b677a16302a9ec1ed81b19

# 能力库分页与执行绑定展示

能力库曾固定每页 10 条，选择 100 后未更新；执行类型取自历史能力分类，列表缺少实际执行名称。

## 改动

- 分页支持 10、20、50、100 条，切换条数返回第一页；刷新和保存保留分页选择，搜索和返回全部重置页码，删除后自动调整到有效页。手机宽度下保留条数选择。
- 按能力 ID 复用工作台执行目录，列表和搜索结果展示实际绑定的执行类型、执行名称；刷新、保存、删除同步更新目录。读取失败显示错误与重试入口，未配置目标才显示“未绑定”；过期读取不能覆盖新数据。
- 类型与名称完整换行，连续英文也能折行；描述继续限宽省略，窄屏只在表格内部横向滚动。

范围限于能力库前端及直接关联的工作台目录读取，没有新增接口或修改数据库数据。

## 功能与测试

| 功能 | 测试 |
| --- | --- |
| 121 条数据按 100 条分页、其余条数切换 | [brainPanel.pagination.dom.test.jsx](../../../crates/web/spa/src/brainPanel.pagination.dom.test.jsx)：`shows 100 rows and the remaining 21, and resets to page one for each page size` |
| 刷新、编辑、搜索与删除后的分页 | 同文件：`retains page size and page after refresh and editing, and resets search pages`、`clamps the last page after deleting its only row and keeps the selected size` |
| 真实绑定、搜索关联、更新与错误重试、旧请求失效 | [capabilities.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/tests/capabilities.dom.test.jsx)：`joins actual bindings by ID for list and search without adding aggregate-only rows`、`refreshes bindings after refresh, saving a new target and deleting a capability`、`shows a binding read failure instead of unbound and retries independently of other lists`、`ignores superseded catalog reads and aborts pending reads on unmount` |
| 原有增删改查与返回工作台 | [brainPanel.dom.test.jsx](../../../crates/web/spa/src/brainPanel.dom.test.jsx)、[navigation.dom.test.jsx](../../../crates/web/spa/src/brain/workbench/tests/navigation.dom.test.jsx) |
| 长名称换行、条数选择、表格内部滚动与详情抽屉 | [layout.js](../../../scripts/acceptance/brain/ui/layout.js) 的 `--only-capabilities` 范围 |

定向 Vitest 最终通过 4 个文件、21 个用例。首轮 4 个用例因按钮中文自动加空格导致测试定位失败；修正选择器后，重跑受影响的两个文件，11 个用例全部通过。其余两个文件的 10 个用例已通过且代码未再变化，未重复运行。未执行全量测试。

最终 SPA 构建通过：在 `crates/web/spa` 运行 `npm run build -- --outDir /tmp/opencoder-brain-library-build-final`，产物独立保存。构建保留现有单包体积提示。

运行 `node scripts/acceptance/brain/ui/layout.js /tmp/opencoder-brain-library-layout-final /tmp/opencoder-brain-library-build-final --only-capabilities`，1920、1280、768、390 像素宽度共 16 项检查通过；每种宽度实际选择 100 条/页，并保存列表、横向滚动、搜索和详情抽屉截图及测量数据。长名称完整折行、描述省略、外层无横向溢出。仅使用虚构显示数据，没有发布线上服务。

## 关联说明

[前端逻辑](../../../agents/web/index.md)、[大脑调度能力](../../brain/index.md)。
