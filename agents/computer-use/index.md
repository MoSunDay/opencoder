Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# computer-use 模块

可选 Python 包 `opencoder-computer`：Cua 桌面操作 CLI。模型循环、动作解析与
执行交给 Cua；本模块只管配置、运行边界与文件产物。

## 代码索引

- [README](../../tools/computer-use/README.md) — 安装与用法
- [cli.py](../../tools/computer-use/src/opencoder_computer/cli.py) —
  `doctor/run/status/cancel` 命令与单行 JSON 输出
- [config.py](../../tools/computer-use/src/opencoder_computer/config.py) —
  纯配置校验（端点、桌面模型、凭证文件）
- [backend.py](../../tools/computer-use/src/opencoder_computer/backend.py) —
  Cua Computer Server 连接与 `ComputerAgent.run`
- [model.py](../../tools/computer-use/src/opencoder_computer/model.py) —
  可选原生函数请求与截图坐标说明
- [runner.py](../../tools/computer-use/src/opencoder_computer/runner.py) —
  动作回调、取消与信号处理
- [state.py](../../tools/computer-use/src/opencoder_computer/state.py) —
  结果、JSONL 事件与截图落盘
- [locks.py](../../tools/computer-use/src/opencoder_computer/locks.py) —
  URL 归一化与本地文件锁
- [results.py](../../tools/computer-use/src/opencoder_computer/results.py) —
  响应归一化、凭证遮盖与退出码
- [ops/](../../tools/computer-use/ops/README.md) — Windows 会话服务安装与转发运维
- [ops/windows/artifacts.ps1](../../tools/computer-use/ops/windows/artifacts.ps1)、
  [archives.ps1](../../tools/computer-use/ops/windows/archives.ps1) — 安装文件与
  归档校验
- `tests/` — CLI / SDK 集成 / 原生模型测试
- CLI 注册与注入范围见
[computer_cli_registration.rs](../../crates/core/tests/computer_cli_registration.rs)

## 边界

- 凭证由配置引用文件，运行结果留在指定目录；无数据库表或核心环境变量。
- CLI 不启停远端服务，也不实现桌面驱动或虚拟机管理。

用户行为和限制见 [computer use](../../features/computer-use/index.md)。
