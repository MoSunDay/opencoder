Commit: 06c69a687d4e5b8416df9376a48f112bd8eb2618

# chat 页白底面板贴底（消除内容区底部 gap）

## 现象与修复

- Operator 对话页（chat）的右侧内容白底面板（`.fleet-sheet`）底部与视口之间有一条 `.fleet-content` 的 20px `padding-bottom` 灰色缝隙；用户要求内容展示区底部贴合屏幕底部。
- 修复：SHEET_PAGES（当前仅 chat）页面的 `Content` 挂 `fleet-content--flush` 修饰类——pane 去掉自身底部 padding、sheet 方角化（去掉左下/右下圆角），面板贴住视口底边；其余 Card 型页面保持 20px 内边距不变。CSS 契约注释同时锚定 main.jsx 的 `SHEET_PAGES`。

## 实现

- `main.jsx`：`SHEET_PAGES.has(shownPage)` 时 `<Content className="fleet-content fleet-content--flush">`，注释同步改写（sheet 停靠语义）。
- `app.css`：`.fleet-content--flush { padding-bottom: 0 }` + `.fleet-content--flush .fleet-sheet { border-bottom-left/right-radius: 0 }`，紧跟 `.fleet-sheet` 定义。

## Validation

- 真实浏览器测量（playwright-core + /usr/bin/chromium，1440x900，fixture 服务复用 `scripts/acceptance/spa_responsive_fixtures.js`）：修复前 sheet bottom=880（差 20px），修复后 sheet bottom=900.0 与 `innerHeight` 精确相等；像素采样 y=890..899 全白，最底一行为 sheet 自身 1px 边框。
- 契约测试：`app.dom.test.jsx` 新增 2 例——chat 页 `.fleet-content` 带 `fleet-content--flush` 且面板包在 `.fleet-sheet` 内；nodes 页无 flush 类、无 sheet 包裹。
- SPA 全量 vitest：112 文件 816 用例通过；`npm run build` 重建 dist，`check-spa-drift.sh` 无漂移。

## Related Docs

- [web 模块](../../../agents/web/index.md)
