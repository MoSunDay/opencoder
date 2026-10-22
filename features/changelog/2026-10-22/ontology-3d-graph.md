Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# Ontology 三维图谱

图谱改用三维空间网络，保留观测范围、命名切面、完整列表和详情编辑。实体管理及服务端 API 保持兼容。

## 行为与实现

- React Force Graph 与 Three.js 替换 G6。节点按类型着色，中心和选中项突出显示；多重、双向关系使用独立曲线和方向箭头。
- 相机支持旋转、缩放、平移、重置视角、概览和中心定位。首次布局完成后取景，切面切换清理旧定位请求；相同结果刷新复用布局，展开及容器调整保留视角。
- HTML 标签保留可访问的按钮、类型和完整关系名称，作为普通文本渲染。标签避开已有文字，较多节点使用紧凑标签，短引线标明被移动标签的实际位置。
- 初始预算仍为 30 个节点、60 条关系，完整列表支持搜索和补入路径后定位。隐藏页签、列表模式和后台页面暂停绘图，卸载释放资源；WebGL 初始化失败或连接中断时回退列表并支持重试。
- 三维坐标仅属于前端模拟对象，不修改 API 查询记录，不新增数据库字段。原 Ontology 能力登记及 CLI/API 入口仍准确。
- 构建继续输出固定的 app.js/app.css，保留 Server 内嵌资源约定。现有二维依赖及专用运行时已移除。

## 测试覆盖

| 功能 | 用例 | 位置 |
| --- | --- | --- |
| 列表定位、中心定位、刷新与渲染失败回退 | `3D graph workspace` | `GraphCanvas.test.tsx` |
| 布局完成、旧回调、切面切换、加载结束与资源释放 | `3D scene lifecycle` | `graphLifecycle.test.tsx` |
| 数据隔离、相机边界、双向曲线和箭头 | `3D scene data and geometry`、`3D camera` | `canvas3d/__tests__/model.test.ts` |
| 完整标签、文本安全和避让 | `complete 3D relationship labels`、`keeps relationship names outside node labels and inside narrow viewports` | `relationshipLabel.test.tsx` |
| 查询、切面、展示预算、记忆及详情 | 既有图谱相关定向用例 | 图谱测试目录及 projection/session |
| 真实三维操作、触摸、40 节点展开、故障回退和屏宽 | 三维浏览器验收 | `scripts/acceptance/ontology/graph.py` |
| 深浅主题、长标签和无 WebGL | 图谱独立显示验收 | `scripts/acceptance/ontology/graphVisual.mjs` |

- SPA 工作目录：`crates/web/spa`。`npm run typecheck:ontology` 通过。
- `npx vitest run src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx src/ontology/pages/graph/__tests__/graphLifecycle.test.tsx src/ontology/pages/graph/__tests__/relationshipLabel.test.tsx --reporter=dot`：21 passed / 0 failed。
- 三维数据与相机模型 4 个、既有查询/切面/筛选/关系详情与展示预算 43 个、观测记忆 2 个、草稿保护 2 个、Ontology 导航标题 4 个定向用例分批通过。共覆盖 76 个不同用例；未选中的其他测试未运行。
- `npm run build` 通过；保留单文件构建的体积提示。
- `cargo build -p opencoder-server --bin opencoder-server` 通过。使用最终 Server 的内嵌页面完成隔离浏览器验收：旋转、缩放、详情、切面、搜索定位、刷新保持视角、触摸、查询错误、WebGL 中断重试及 40 节点展开均通过。1920、1280、768、390 像素宽度无页面横向溢出。
- 验收逐字节核对实际响应的 index.html、app.js、app.css 与本地构建一致，记录哈希及 Server 构建信息。回执和截图位于 `/tmp/ontology-3d-final-20261022-0710/`，`receipt.json` 为 `passed: true`；验收使用临时数据目录，结束后已关闭临时 Server。
- 独立显示检查通过：深浅主题 × 1280/390 像素宽度，长关系名称完整换行，无 WebGL 时保留完整列表。
- 隔离 Server 的 `GET /api/ontology/envs/debug/sync/check` 返回 `in_sync: true`。
- 未修改 Rust；Clippy 不适用。未运行全量测试，未发布生产。

## 相关

- [功能说明](../../ontology/index.md)
- [前端模块记忆](../../../agents/web/ontology.md)
