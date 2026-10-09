Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# Ontology 前端

TypeScript + React + antd 页面通过平台请求访问现有 Ontology API。图谱、详情与表单分别管理状态；环境和实体身份参与异步结果校验。

## 入口与共享状态

- [panels.tsx](../../crates/web/spa/src/ontology/panels.tsx)、[env.tsx](../../crates/web/spa/src/ontology/env.tsx)、[api.ts](../../crates/web/spa/src/ontology/api.ts) 连接五个导航页面、环境和平台身份。写权限来自服务端能力，正文保存回执提供新 revision。
- [DraftGuard.tsx](../../crates/web/spa/src/ontology/navigation/DraftGuard.tsx) 在 Shell 内登记草稿和保存状态，供关闭、导航、环境切换、退出及浏览器离开检查使用。[ModalForm.tsx](../../crates/web/spa/src/ontology/ui/ModalForm.tsx) 统一表单提交、错误保留与放弃确认。
- [admin/](../../crates/web/spa/src/ontology/pages/admin/) 提供列表筛选、资源读取状态和危险操作菜单；[DirectoryActions.tsx](../../crates/web/spa/src/ontology/pages/DirectoryActions.tsx) 根据目录类型标记连接目录接口。

## 图谱

- [GraphPage.tsx](../../crates/web/spa/src/ontology/pages/graph/GraphPage.tsx)、[useGraphObservation.ts](../../crates/web/spa/src/ontology/pages/graph/useGraphObservation.ts) 组织元数据、观测请求和最后成功结果，图谱请求固定启用 `expandNeighbors`；[aspects/](../../crates/web/spa/src/ontology/pages/graph/aspects/) 连接命名切面与自定义范围。
- [observationMemory.ts](../../crates/web/spa/src/ontology/pages/graph/session/observationMemory.ts) 按环境保存成功的范围与命名切面，恢复前校验类型、关系和中心是否仍有效；观测记录不保存邻居展开选项。
- [projection/model.ts](../../crates/web/spa/src/ontology/pages/graph/projection/model.ts) 用纯函数选择画布节点和边，保留实际连接路径；[useProjection.ts](../../crates/web/spa/src/ontology/pages/graph/projection/useProjection.ts) 管理展示预算。[ResultList.tsx](../../crates/web/spa/src/ontology/pages/graph/projection/ResultList.tsx) 使用完整查询结果。
- [GraphCanvas.tsx](../../crates/web/spa/src/ontology/pages/graph/GraphCanvas.tsx) 组织 G6；[canvas/](../../crates/web/spa/src/ontology/pages/graph/canvas/) 管理实例、尺寸变化、视口、工具栏和关系标签测量。画布展示预算与查询结果分开，缩放及容器尺寸变化不重建观测范围；单个观测中心的定位按钮一次点击直接执行，复用同一 G6 实例切换切面时等待新数据布局完成后重新渲染并把中心恢复到 100% 居中。
- [details/](../../crates/web/spa/src/ontology/pages/graph/details/) 保存实体和关系详情的返回路径、标签及滚动位置，并支持以当前实体重新观测。

## 实体详情

- [EntityDetailDrawer.tsx](../../crates/web/spa/src/ontology/pages/entityTypes/EntityDetailDrawer.tsx) 组织基本信息、普通属性、拓展信息、来源四个标签；正文按所选标签读取，草稿状态留在详情层。
- [useStructuredDrafts.ts](../../crates/web/spa/src/ontology/pages/entityTypes/details/useStructuredDrafts.ts)、[useTextDrafts.ts](../../crates/web/spa/src/ontology/pages/entityTypes/details/useTextDrafts.ts) 分别维护结构化值和正文的原值、草稿、版本与保存状态；环境、实体和请求代次共同阻止旧结果覆盖当前内容。提交成功后采用回执中的新版本，刷新失败仍保留已提交内容。
- [TextPreview.tsx](../../crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.tsx) 区分 Markdown 与 HTML；HTML 使用隔离 iframe 和 CSP，源码编辑保留原格式。NFS 路径输入与读取出的正文分别保存。

## 验证与接缝

- 单元及 DOM 测试与上述模块相邻；[testSetup.ts](../../crates/web/spa/src/ontology/testSetup.ts) 提供浏览器环境适配。
- [Ontology 浏览器验收](../../scripts/acceptance/ontology/main.py)、[详情与图谱操作](../../scripts/acceptance/ontology/experience.mjs) 使用独立 Server 和真实只读 NFS，检查草稿、图谱控制、恢复及导航标签。
- [Ontology 服务](../ontology/index.md) · [功能规则](../../features/ontology/index.md) · [Web 索引](index.md)
