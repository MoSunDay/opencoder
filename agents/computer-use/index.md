Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# computer-use 模块

可选 Python 包 [`tools/computer-use`](../../tools/computer-use/README.md)，提供独立 `opencoder-computer` CLI。通过 [core 的 CLI 注册](../core/index.md)向主代理注入使用说明，沿用会话 bash。调用方有图片查看工具时可检查 PNG。Cua 拥有模型循环和桌面动作；本模块管理配置、运行边界和文件产物。

## 代码索引

- [cli.py](../../tools/computer-use/src/opencoder_computer/cli.py)：`doctor/run/status/cancel` 参数与单行 JSON 输出；SDK 诊断走遮盖凭证后的 stderr。
- [config.py](../../tools/computer-use/src/opencoder_computer/config.py)：纯配置校验；目标系统、HTTP(S) 端点、独立桌面模型及凭证文件读取。
- [backend.py](../../tools/computer-use/src/opencoder_computer/backend.py)：连接 Cua Computer Server、调用 `ComputerAgent.run`；适配系统信息、服务端失败回执和连接关闭。Windows 输入使用 Cua 的剪贴板及粘贴命令。
- [model.py](../../tools/computer-use/src/opencoder_computer/model.py)：可选原生函数请求、当前截图及坐标说明、执行前响应校验；只读模型检查仍使用 Cua 的预测与坐标转换。
- [runner.py](../../tools/computer-use/src/opencoder_computer/runner.py)：动作回调、超时、取消和信号处理；任务前后截图及最终状态。
- [state.py](../../tools/computer-use/src/opencoder_computer/state.py)：原子写入结果、JSONL 事件、PNG 截图、取消请求；根据运行锁识别异常退出。
- [locks.py](../../tools/computer-use/src/opencoder_computer/locks.py)：URL 归一化和系统文件锁，同一本地锁目录内防止并发操作同一端点。
- [results.py](../../tools/computer-use/src/opencoder_computer/results.py)：响应归一化、凭证遮盖、终端摘要和退出码。
- [Windows 安装文件校验](../../tools/computer-use/ops/windows/artifacts.ps1)与[归档校验](../../tools/computer-use/ops/windows/archives.ps1)：先校验完整 manifest 和目标路径，再创建目录或下载；SHA256 校验通过后替换文件。拒绝安装根外路径、Windows 路径别名、重解析点及归档链接，解包前检查全部成员。原生文件系统覆盖见 [artifact-tests.ps1](../../tools/computer-use/ops/windows/artifact-tests.ps1)。

## 依赖与边界

Cua SDK 源码提交固定在包依赖中。服务端可以使用上游原生后端或其匹配的可选 Driver。固定提交的 SDK 内部接口由 [SDK 集成测试](../../tools/computer-use/tests/test_sdk.py)和 [原生模型测试](../../tools/computer-use/tests/test_native_model.py)约束。模型、动作解析及执行交给 Cua，不实现桌面驱动或虚拟机管理。

CLI 本身不启停远端服务；独立的 [运维脚本](../../tools/computer-use/ops/README.md)用于安装 Windows 用户会话服务及维护受保护的转发。转发恢复不重放任务。接入范围为本机 CLI/TUI，远端 DAG/runc 的依赖与配置分发由部署方另行准备。

凭证由配置引用文件；运行结果留在指定目录。没有数据库表或 OpenCoder 核心环境变量。CLI 调用与注入范围由 [注册测试](../../crates/core/tests/computer_cli_registration.rs)验证，进程行为见 [CLI 测试](../../tools/computer-use/tests/test_cli.py)。

用户行为和限制见 [computer use](../../features/computer-use/index.md)。
