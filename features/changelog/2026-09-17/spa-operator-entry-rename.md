Commit: 0657c339

# SPA「会话交互」入口更名 Operator

该页以 operator kind 创建会话（`newId('operator')` + `POST /api/sessions`，见 control `session.rs::create`），本就是 Operator 的入口；仅改用户可见标签，页面 key 保持 `chat`（深链与 store 兼容）。

## 变更

- `crates/web/spa/src/nav.js`：IA 单一事实源中 `chat` 页 label「会话交互」→「Operator」。
- `crates/web/spa/src/operators/panel.jsx`、`nodeTable.jsx`：页签只读说明与注释同步；`src/chat.jsx` 头注释补 Operator entry 说明。
- 测试同步：`nav.test.js`、`app.dom.test.jsx`、`operators/panel.dom.test.jsx`。

## 测试覆盖

| 功能 | 测试 | 结果 |
|------|------|------|
| SPA 全量 | vitest run | 109 文件 / 799 tests 全绿 |
| dist 漂移门 | scripts/check-spa-drift.sh | no drift |
