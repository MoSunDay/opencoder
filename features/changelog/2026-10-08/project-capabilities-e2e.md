# DAG 派发与原生验收

- 从 TODO 打开 DAG 派发界面时独立读取节点列表，不再依赖先访问节点页；读取失败在派发抽屉中显示。
- 动态 DAG 验收等待整个运行终态后停止节点；响应式图谱验收改变屏宽前关闭悬浮提示。

验证入口：`loads dispatch nodes without first visiting the fleet page`、`shows node fetch failures in the dispatch drawer`、`scripts/acceptance/dag_dynamic.js`、`scripts/acceptance/ui/main.js`。
