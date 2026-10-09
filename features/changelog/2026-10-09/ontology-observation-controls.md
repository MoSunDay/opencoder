Commit: bc94554ff6ce5ccae0fd23702f01641f1608e27b

# Ontology 图谱自动展示关联实体

## 背景

关联实体展示由页面自动处理，无需用户理解或操作“展开跨类型邻居”开关。

## 变化与影响

- 删除该开关，自定义范围和命名切面查询均自动展示范围内关联的其他类型实体。
- 类型、关系、中心和上下游跳数仍可调整；新打开图谱默认进入自定义范围，已有成功观测可恢复。
- 观测记录不再保存邻居展开选项；已有记录中的旧值不影响自动展示。
- 服务端图谱接口保持原有能力，无数据库或配置变更。

## 验证

- SPA 全量 151 个测试文件、1074 个测试通过，类型检查及构建通过。
- 正式 UI 验收覆盖全部 16 个范围及 TUI；线上浏览器检查 1920、1280、768、390 四种宽度，确认开关不存在且无浏览器错误。
- [GraphPage 测试](../../../crates/web/spa/src/ontology/pages/graph/__tests__/GraphPage.test.tsx) 覆盖自动查询、成功记录保存和旧记录恢复；[命名切面测试](../../../crates/web/spa/src/ontology/pages/graph/__tests__/GraphPageAspects.test.tsx) 验证查询参数。

## 相关

- [Ontology 功能](../../ontology/index.md)
- [Ontology 前端](../../../agents/web/ontology.md)
