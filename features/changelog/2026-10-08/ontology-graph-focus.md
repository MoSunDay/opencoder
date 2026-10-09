Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 图谱中心定位与切面切换交互修复

- “定位观测中心”在只有一个观测中心时一次点击直接定位，不再先打开候选框。
- 切换切面时复用已有 G6 实例，等新数据布局完成后重新渲染、恢复 100% 缩放并把观测中心移到画布中央。
- `GraphCanvas` 图谱回归测试覆盖了直接定位与切面切换，ontology 类型检查通过。

相关：[Ontology 前端逻辑](../../../agents/web/ontology.md)、[Ontology 功能](../../ontology/index.md)。
