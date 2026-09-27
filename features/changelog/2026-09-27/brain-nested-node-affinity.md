Commit: 1565c74a3f348aeb0d550a2aae6bdcdabaf52ff5

# 嵌套 Brain 沿用父节点

里程碑调度派发嵌套计划时将父运行的节点身份传入子运行准入。子计划仍固定保存版本并验证父操作，避免被全局选点派到无关专用节点；非 PC 的普通能力节点继续走全局选点。

[大脑调度工作台](../../brain/index.md) · [Control 模块](../../../agents/control/index.md)
