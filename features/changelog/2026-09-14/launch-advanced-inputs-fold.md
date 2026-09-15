Commit: 3908926718bbcaec47614743e1228a91c40c4362 (working-tree)

# 大脑发起表单：可选初始输入收进「高级选项」折叠区

新任务主表单此前平铺「初始输入（JSON）」裸 textarea，用户易误读为必填 JSON；实际必填仅「目标和交付物」+「大脑所在节点」。本轮把它收进「高级选项」折叠区：标签去掉 JSON 字样（「初始输入（可选）」），tooltip 说明用途（为计划已声明的输入端口预填参数），placeholder「留空即可，运行中会按需询问」；默认值从 `{}` 改为空串使 placeholder 可见（`launchBody` 对空串仍回落 `{}`）。一句话零预填发起路径与 `waiting_input` 按需询问不受影响。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 折叠区默认隐藏初始输入、展开后可见且空值照常发起 | `folds the optional prefill inputs behind advanced options and launches a one-liner without them` | `crates/web/spa/src/brain/workbench/tests/launch.dom.test.jsx` |
| 一句话发起 durable 提交语义（回归） | `keeps a durable run identity across an uncertain submission and exposes errors` | 同上 |
| launchBody 空串回落 `{}`、非法 JSON 抛错（回归） | `dynamic planning uses explicit references and fixed mode pins an exact version` | `crates/web/spa/src/brain/workbench/tests/model.test.js` |

- SPA 全量回归：`npm test`（vitest）→ 693 passed / 0 failed（89 文件）
- `npm run build` 成功；`crates/web/spa/dist/static/app.js` 随构建更新

## Related Docs

- [features/brain](../../brain/index.md)（交互路径第 1 步已同步）
