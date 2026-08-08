# TUI 标题与 mode 重排

## 变更

- body 顶部标题改为 `workdir · model · thinking level`，模型名称和思考档位紧邻 workdir，不再右对齐或在窄终端主动丢弃。
- `[mode]` 从 body 标题移到底部状态栏左下角，保留 act/plan 对应的主题色。
- subagent 视图的返回/导航标题不变；底部 mode 取当前 `ChatView.agent`。

## 验证

- 纯函数标题组合测试覆盖 model/effort 相邻排列与空 effort 省略。
- 状态栏渲染测试锁定 `[act]` 位于首个可见左侧位置，同时保留 ctx、spinner/status 和耗时。
