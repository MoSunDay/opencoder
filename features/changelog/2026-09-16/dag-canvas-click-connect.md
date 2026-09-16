Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# DAG 画布点击步骤即可连线

## 背景

工作流定义编辑器的「画布」模式此前只能从步骤卡片左右两侧的小圆点 Handle **拖拽**建立依赖边：用户反馈「没法点击 step 去连接 step」。点击卡片本体只会选中（开右侧 Inspector），Handle 仅 6px 且无任何点击式连线引导；另外从 Handle 拖到非法目标时 `isValidConnection` 静默拦截，成功与否全凭猜。运行视图（`process.jsx`）`nodesConnectable={false}` 属于有意设计（点节点打开单步记录抽屉），不在本次范围。

## 变更（全部在编辑器画布，运行画布只读语义不变）

- `crates/web/spa/src/dag/editor/canvasEditor.jsx`
  - 新增连线模式状态 `linkMode`（工具栏开关）与待连接源 `linkFrom`；`onNodeClick` 实现两段式点击连线：armed 后点击另一节点即经 `canConnect` 守卫走既有 `onConnect` 落边（拒自连/重复/成环，中文 warning），点源自身/点空白/Esc 取消。
  - `onConnect` 改为返回 boolean 供点击链路判定；新增 `onConnectEnd`：Handle 点击未拖动（松手无目标）自动进入待目标模式；拖拽落在非法 Handle 被拒时弹出 `canConnect` 原因，不再静默。
  - linkbar 提示条（armed 显示「连线：源 → 点击目标步骤（Esc 取消）」，未选源显示模式说明）。
  - 节点 data 注入 `linkSource`/`linkTarget`（合法目标 = `canConnect === null`），与既有 meta effect 合并互不覆盖。
- `crates/web/spa/src/dag/editor/canvasToolbar.jsx`：新增「连线」toggle 按钮（激活态 primary，title 说明）。
- `crates/web/spa/src/dag/editor/stepNode.jsx`：`data.linkSource/linkTarget` → 卡片高亮 class + hover title。
- `crates/web/spa/src/app.css`：Handle 放大至 10px；`.dag-edit-node--linksrc`（脉冲描边）、`--linktgt`（crosshair + hover 描边）、`.dag-edit-linkbar`。
- 拖拽 Handle 连线与 JSON 模式编辑 deps 全部保留，`canvasToSpec` 由入边重建 `depends_on` 的契约不变。

## Impact Surface

- 修改 `crates/web/spa/src/dag/editor/{canvasEditor.jsx,canvasToolbar.jsx,stepNode.jsx}`、`crates/web/spa/src/app.css`
- 测试 `crates/web/spa/src/dag/editor/editor.dom.test.jsx` 新增「画布连线模式」4 用例
- 重建产物 `crates/web/spa/dist/static/{app.js,app.css}`（无 src↔dist 漂移，`check-spa-drift.sh` 1/3 通过）
- Rust / 协议零改动

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 连线模式两段点击建边、armed 高亮与提示条、保存后 `depends_on` 正确 | `连线模式点击两个步骤建立依赖并保存` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 成环依赖被拒且 spec 不变（warning 提示） | `连线模式拒绝成环依赖且不改 spec` | 同上 |
| Esc 取消待连接源、高亮回落 | `Esc 取消待连接的源节点` | 同上 |
| 未开连线模式点击节点仍是选中（不武装连线） | `未开连线模式时点击节点仍是选中（不武装连线）` | 同上 |
| canConnect 守卫（自连/重复/成环） | 既有 `canConnect` 纯函数用例 | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 运行画布只读语义（点节点开抽屉、禁连线）不回退 | `shows final states immediately, opens the step drawer and loads run logs behind it` | `crates/web/spa/src/dag/graph.dom.test.jsx` |

## 后续修正（本提交）

实现主体（canvasEditor/canvasToolbar/stepNode/app.css/dist）已随 30108c8b 进库，但收编的测试用例方向有误：

- 「连线模式点击两个步骤建立依赖并保存」连 `review→fetch`——与既有 `fetch→review` 边成环被 `canConnect` 拒绝，用例必红；
- 「连线模式拒绝成环依赖且不改 spec」连 `fetch→review` 只会命中「依赖已存在」，其 `不能形成循环依赖` 断言靠上一用例残留的 message DOM 假绿。

本提交把用例 1 改为连到新添加的 wasm 步骤（先补 command 过校验），用例 2 改为 `review→fetch` 真成环方向，两用例断言自此真实成立；并重建 `dist/static/{app.js,app.css}`（30108c8b 收编的 dist 与 src 存在 DRIFT，重建后 no drift）。

## 验证

- `npx vitest run src/dag/editor/editor.dom.test.jsx` → 11 tests 全绿（HEAD 版本实测 1 failed）
- `npm test` → 108 files / 798 tests 全绿
- `bash scripts/check-spa-drift.sh` → no drift
