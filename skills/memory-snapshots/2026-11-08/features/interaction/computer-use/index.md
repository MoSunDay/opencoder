Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# Computer use

可选 Python 包 `opencoder-computer`：Cua 桌面操作 CLI，由 Cua Agent 经
`/cli` 注册使用；本卡只做指路。

## 索引

- [使用文档](../../../tools/computer-use/README.md) — 安装、配置与方案调研
- [pyproject.toml](../../../tools/computer-use/pyproject.toml) —
  `opencoder-computer` 包定义
- [cli.py](../../../tools/computer-use/src/opencoder_computer/cli.py)
  — `doctor`/`run`/`status`/`cancel` 命令入口
- [runner.py](../../../tools/computer-use/src/opencoder_computer/runner.py)、
  [state.py](../../../tools/computer-use/src/opencoder_computer/state.py)、
  [results.py](../../../tools/computer-use/src/opencoder_computer/results.py) —
  运行循环、状态与结果/截图产物
- [config.py](../../../tools/computer-use/src/opencoder_computer/config.py)、
  [model.py](../../../tools/computer-use/src/opencoder_computer/model.py)、
  [locks.py](../../../tools/computer-use/src/opencoder_computer/locks.py)、
  [backend.py](../../../tools/computer-use/src/opencoder_computer/backend.py) —
  配置（`native_tool_calls`）、桌面模型、单实例锁与后端
- [运维说明](../../../tools/computer-use/ops/README.md)、
  [install-windows.ps1](../../../tools/computer-use/ops/install-windows.ps1) —
  Windows 服务安装
- [computer-use 模块](../../../agents/execution/computer-use/index.md) — 实现索引域卡

## 相关

- [使用文档](../../../tools/computer-use/README.md)、
  [实现索引](../../../agents/execution/computer-use/index.md)
