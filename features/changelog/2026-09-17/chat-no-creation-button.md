Commit: d54758a5c8e12f7a505e05c24748213eb2e84db5

# 对话页移除「新建对话」按钮——选中节点提交需求即新建对话

## 交互

- 会话侧栏不再有「新建对话」入口：`Conversations` 的 `creation` prop 移除。
  创建已由发送链路承担——选中节点后首次发送提示词即 `POST /api/sessions`
  建会话（`chat/nodeSelection.dom.test.jsx` 既有契约），按钮只是重复入口。
- 空态文案「选择或新建对话，输入提示词开始」→「选中节点后输入提示词，即
  新建对话」；compact/fork/autopilot/annotation 四处无会话提示「先选择或
  新建对话」→「先发送一条提示词新建对话」（发送即可满足前置）。
- Operator/Agent 模式切换（页头「会话模式」Segmented）不受影响：Agent 模式
  创建 body 带 `kind:'agent'` + 可选注册 agent（@ 菜单暂存到 `body.agent`）。

## 实现

- `chatSidebar.jsx`：删 `creation` prop 与 `onNew` prop。
- `chat.jsx`：删 `onNew` handler（原 resetTranscript + setDialogSel(null) +
  setSessionAgent('act') + 清 createAttempt——切节点 effect 已覆盖同一状态
  收敛，无独立 handler 需要）。
- dist 重建（`npm run build`），Rust 编译期内嵌随下次构建生效。

## 测试

- `sidebar.dom.test.jsx`：「starts a new chat from the creation button」改写
  为「has no dedicated creation button」——断言 creation 按钮不存在 + 切换到
  无会话节点回落空态新文案。
- `app.dom.test.jsx` 两处空态断言同步新文案。
- 回归：SPA vitest 全量 113 文件 827 用例通过。

## Related Docs

- [web 模块](../../../agents/web/index.md)

## Release

- `d54758a5` 落 main（未发布）。
