Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# Ontology 前端

TypeScript + React + antd 页面通过平台请求访问现有 Ontology API。图谱、详情与表单分别管理状态；环境和实体身份参与异步结果校验。

## 入口与共享状态

- [panels.tsx](../../crates/web/spa/src/ontology/panels.tsx)、[env.tsx](../../crates/web/spa/src/ontology/env.tsx)、[api.ts](../../crates/web/spa/src/ontology/api.ts) 连接四个导航页面与平台身份。页面固定使用 Server 初始化的 `debug` 数据空间，由平台请求地址确定 Server；不读取旧环境选择、不查询环境列表，也不暴露环境管理。写权限来自登录身份的角色，正文保存回执提供新 revision。
- [DraftGuard.tsx](../../crates/web/spa/src/ontology/navigation/DraftGuard.tsx) 在 Shell 内登记草稿和保存状态，供关闭、导航、退出及浏览器离开检查使用。[ModalForm.tsx](../../crates/web/spa/src/ontology/ui/ModalForm.tsx) 统一表单提交、错误保留与放弃确认。
- [admin/](../../crates/web/spa/src/ontology/pages/admin/) 提供列表筛选、资源读取状态和危险操作菜单；[DirectoryActions.tsx](../../crates/web/spa/src/ontology/pages/DirectoryActions.tsx) 根据目录类型标记连接目录接口。

## 图谱

- [GraphPage.tsx](../../crates/web/spa/src/ontology/pages/graph/GraphPage.tsx)、[useGraphObservation.ts](../../crates/web/spa/src/ontology/pages/graph/useGraphObservation.ts) 组织元数据、观测请求和最后成功结果，图谱请求固定启用 `expandNeighbors`；[aspects/](../../crates/web/spa/src/ontology/pages/graph/aspects/) 连接命名切面与自定义范围。
- [observationMemory.ts](../../crates/web/spa/src/ontology/pages/graph/session/observationMemory.ts) 按环境保存成功的范围与命名切面，恢复前校验类型、关系和中心是否仍有效；观测记录不保存邻居展开选项。
- [projection/model.ts](../../crates/web/spa/src/ontology/pages/graph/projection/model.ts) 用纯函数选择画布节点和边，保留实际连接路径；[useProjection.ts](../../crates/web/spa/src/ontology/pages/graph/projection/useProjection.ts) 管理展示预算。[ResultList.tsx](../../crates/web/spa/src/ontology/pages/graph/projection/ResultList.tsx) 使用完整查询结果。
- [GraphCanvas.tsx](../../crates/web/spa/src/ontology/pages/graph/GraphCanvas.tsx) 组织三维画布、完整列表、展示预算和渲染失败回退；[canvas3d/](../../crates/web/spa/src/ontology/pages/graph/canvas3d/) 使用 React Force Graph 与 Three.js，将查询记录转换为独立的模拟对象，管理三维曲线、方向箭头、HTML 标签、相机及资源释放。相同数据刷新复用布局对象，切面和中心切换在布局完成后定位；搜索先补入目标及路径再定位。
- [canvas/](../../crates/web/spa/src/ontology/pages/graph/canvas/) 保留工具栏、视口高度和名称展示规则。缩放、旋转、平移、放大画布及容器尺寸变化不改变查询范围；隐藏场景暂停绘制，渲染失败保留完整列表并允许重试。标签通过三维坐标投影定位，名称作为普通文本渲染，关系名称完整换行。
- [details/](../../crates/web/spa/src/ontology/pages/graph/details/) 保存实体和关系详情的返回路径、标签及滚动位置，并支持以当前实体重新观测。

## 实体详情

- 实体详情可返回 `management`，标明平台生成的数据与原模块 CLI/API 入口。生成实体的基本信息、来源和定义属性只读；补充说明与自定义属性沿用编辑流程。同步生成的能力直接进入已有实体页和图谱切面。

- [EntityDetailDrawer.tsx](../../crates/web/spa/src/ontology/pages/entityTypes/EntityDetailDrawer.tsx) 组织基本信息、普通属性、拓展信息、来源四个标签；正文按所选标签读取，草稿状态留在详情层。
- [useStructuredDrafts.ts](../../crates/web/spa/src/ontology/pages/entityTypes/details/useStructuredDrafts.ts)、[useTextDrafts.ts](../../crates/web/spa/src/ontology/pages/entityTypes/details/useTextDrafts.ts) 分别维护结构化值和正文的原值、草稿、版本与保存状态；环境、实体和请求代次共同阻止旧结果覆盖当前内容。提交成功后采用回执中的新版本，刷新失败仍保留已提交内容。
- [TextPreview.tsx](../../crates/web/spa/src/ontology/pages/entityTypes/details/TextPreview.tsx) 区分 Markdown 与 HTML；HTML 使用隔离 iframe 和 CSP，源码编辑保留原格式。NFS 路径输入与读取出的正文分别保存。

## 验证与接缝

- 单元及 DOM 测试与上述模块相邻；[testSetup.ts](../../crates/web/spa/src/ontology/testSetup.ts) 提供浏览器环境适配。
- [Ontology 浏览器验收](../../scripts/acceptance/ontology/main.py)、[详情与图谱操作](../../scripts/acceptance/ontology/experience.mjs) 使用独立 Server 和真实只读 NFS，检查草稿、图谱控制、恢复及导航标签。[三维图谱验收](../../scripts/acceptance/ontology/graph.py) 单独验证图谱交互、屏宽、切面和 WebGL 故障回退，并比对 Server 实际提供的静态资源。
- [Ontology 服务](../ontology/index.md) · [功能规则](../../features/ontology/index.md) · [Web 索引](index.md)

实体目录页面通过目录成员关系计算所选目录及全部后代的实体集合，先去重，再进行搜索、类型和状态筛选及分页。普通关系不参与目录归属计算。目录选择器显示完整路径；移动目录排除自身和后代。实现与定向测试位于 `spa/src/ontology/pages/entities/directories.ts` 及相邻测试文件。

目录树将 JSON 中的空父级归一化，首次进入只展开根目录，其他层级逐层展开。默认先选择目录再显示实体；桌面与窄屏均可显式点击“全部实体”，包括未归入目录的实体。切换范围重置分页，目录刷新保留用户的展开状态。

观测直达链接由 `ontology/navigation/observationLink.ts` 解析，`graph/session/linkedObservation.ts` 校验切面与中心，复用全局地址监听和草稿保护。链接条件优先于浏览器记忆，不写回切面；离开图谱时移除对应参数。
