Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 改进 Ontology 交互体验

改进 OpenCoder 的图谱、详情和管理页，使用 antd 6、平台身份、现有 Ontology API、SQLite 和本服务的只读 NFS。

## 变更

- 图谱使用紧凑范围选择、完整关系名称、缩放与中心定位；分批绘图与完整结果列表分开，环境分别恢复最近成功观测。
- 实体详情改为四个标签，结构化属性可直接编辑，Markdown/HTML 支持预览与源码编辑；草稿跨标签保留，离开有确认，保存冲突及刷新失败保留正确的内容和版本。
- 类型与关系列表补齐筛选、读取错误和重试；危险操作进入菜单，目录操作按通用类型标记处理。
- 前端按图谱、投影、详情、草稿、管理和导航拆分模块，移除未使用的旧属性编辑器；复用本仓库请求、权限、表单和表格组件。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 关系名称完整显示与布局预留 | `preserves every character and reserves enough space for %s` | [relationshipLabel.test.ts](../../../crates/web/spa/src/ontology/pages/graph/__tests__/relationshipLabel.test.ts) |
| 缩放、刷新与容器尺寸变化 | `preserves the graph and viewport during loading, refresh, and expansion`、`keeps the same graph coordinate at the viewport center after resizing` | [GraphCanvas.test.tsx](../../../crates/web/spa/src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx) |
| 大图分批显示与搜索定位 | `bounds thousand-node data, counts hidden neighbors and keeps the original result intact`、`reveals a hidden search result with its complete real path` | [model.test.ts](../../../crates/web/spa/src/ontology/pages/graph/projection/model.test.ts) |
| 环境内恢复成功观测 | `restores the successful custom observation including cross-type neighbors` | [GraphPage.test.tsx](../../../crates/web/spa/src/ontology/pages/graph/__tests__/GraphPage.test.tsx) |
| 四个详情标签、草稿与 HTML | `preserves structured edits across tabs, confirms close, and blocks closing during a save`、`previews custom HTML in a sandbox, exposes its source, and saves the same format` | [EntityDetailDrawer.test.tsx](../../../crates/web/spa/src/ontology/pages/entityTypes/EntityDetailDrawer.test.tsx) |
| 版本冲突、保存后刷新失败和 NFS 新版本 | `retains a conflicting draft while metadata refreshes and loads the new body on cancel`、`keeps the committed text and revision when refreshing metadata fails`、`reads the newly bound NFS version and preserves the relative path` | [useTextDrafts.test.tsx](../../../crates/web/spa/src/ontology/pages/entityTypes/details/useTextDrafts.test.tsx) |
| 页面离开与失败表单保护 | `protects registered drafts on page navigation and browser unload`、`preserves failed modal submissions and confirms discarding the form` | [DraftGuard.test.tsx](../../../crates/web/spa/src/ontology/navigation/DraftGuard.test.tsx) |
| 实际详情编辑、图谱控制、恢复、环境隔离和只读 NFS | `checkDetails`、`checkGraph` 及浏览器/NFS 回执 | [experience.mjs](../../../scripts/acceptance/ontology/experience.mjs)、[main.py](../../../scripts/acceptance/ontology/main.py) |

## 验证结果

- SPA：147 个文件、1,052 个测试全部通过；Ontology 类型检查、构建及漂移检查通过。共享主机上以 2 个 worker、30 秒测试/初始化预算执行最终全量检查，保留原有断言。
- Rust：`cargo test --workspace` 为 5,700 passed / 0 failed，8 项原有环境用例保持 ignored；格式、全目标 Clippy 零警告与工作区构建通过。
- 发布脚本回归：145 项通过。真实原生 DAG 完成 900 秒、79 次观察采样，取消、失败、动态任务、重启和固定版本恢复通过，进程与挂载清理通过。
- 平滑切换及回滚完成 900 秒观察，301 次提交的最大受理耗时为 0.535 秒，观察阶段完成 57 个任务。
- 全站 16 项 UI/TUI 检查全部通过，包含 1920、1280、768、390 四种屏宽及全部 15 个导航页；Ontology 五页、真实只读 NFS、草稿保护、环境恢复和分类标签横向滚动通过。验收提示浮层等待修正后另行补测 Ontology，通过且清理完成。
- 最终 SPA 摘要为 `60174f65a92d8ec96469ffe2dc4216d241c3d60dc583d7f73bfae447144df5de`，与当前工作区及 UI 验收 Server 一致。验收期间保留并行工作的 DAG 前端与测试脚本修正；1,794 个 Rust/Cargo 文件经摘要核对与全量后端回归时一致。

证据保存在 `/root/.cache/opencoder-e2e/20261007-ontology-experience-sync/`：`final-frontend-result.json`、`frozen-workspace-tests.log`、`frozen-release-result.json`、`ui-final/receipt.json`、`ontology-final-tooltip/result.json`、`final-ui-result.json` 与 `verification.json`。原生观察回执位于同级 `20261007-ontology-experience-frozen-native/evidence/result.json`，切换观察回执位于证据目录的 `opencoder-smooth-l35z5n4a/result.json`。本记录对应开发验收，未执行生产发布。

相关：[Ontology 功能](../../ontology/index.md) · [前端逻辑索引](../../../agents/web/ontology.md) · [Ontology 服务](../../../agents/ontology/index.md)
