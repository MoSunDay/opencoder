Commit: 3fed5f35

# 目录按层级折叠

接口返回根目录的 `parent_id: null`，前端原先只将 `undefined` 识别为根，导致有根目录时退回平铺全部目录。统一空父级后按父子关系构建树，初次进入仅展开根目录，后续层级由用户展开。默认提示选择目录，点击“全部实体”才展示全量列表。

实体标识、属性、正文和普通关系不变；沿用父目录汇总后代、搜索、类型筛选和分页。

## 验证范围

- `directories.test.ts`：JSON 空父级嵌套、缺失父级与循环，以及后代成员计算。
- `CreateEntityModal.test.tsx`：既有实体创建及重试行为。
- 浏览器：首次折叠、展开收起、按目录过滤、全部实体，以及 1920、1280、768、390 宽度。
- Ontology TypeScript 检查、SPA 构建和发布产物核验；未运行全量测试。

相关：[Ontology](../../ontology/index.md)、[前端逻辑](../../../agents/web/ontology.md)。
