Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# Computer use

安装可选 `opencoder-computer` 后，用户可通过 OpenCoder `/cli` 注册，让 Cua Agent 使用独立桌面模型完成已有远程桌面上的自然语言任务。桌面应用和浏览器共用这条入口。

## 使用约定

本次入口为本机 CLI/TUI。CLI 及其 Python 依赖、配置和凭证需要在 bash 工具实际执行的环境中可访问。配置目标系统和连接地址，把完整任务及完成检查写入 UTF-8 文件。先 `doctor --target` 检查桌面，再 `run --target --task-file --output-dir` 启动任务；输出目录必须是新目录，由命令创建。

通过 bash 前台启动任务。超过前台等待期限时，保存 OpenCoder 自动移交后台管理的进程句柄，随后通过 `status --run-dir` 查询；不要重复启动。`cancel --run-dir` 请求停止。

每次命令返回单行 JSON。完整结果、事件和编号 PNG 截图保存在运行目录。调用方有图片查看工具时应检查实际截图，否则返回截图路径供复核，不能仅根据模型摘要声称已完成视觉验证。默认 `doctor` 检查桌面连接和模型循环选择；增加 `--check-model` 会请求真实模型验证视觉、函数格式和坐标，但不执行桌面动作。

桌面模型独立配置。通用视觉模型可启用 `native_tool_calls`，执行前拒绝混合工具文本、非法坐标和并行动作。Windows 中文输入会替换目标桌面的剪贴板内容，然后通过 Cua 粘贴。

## 状态与限制

- `running` 表示仍在执行；`cancel_requested` 只确认已提交取消请求，需要继续查询终态。
- `completed` 表示 Cua 正常返回最终助手结果；是否达成任务应查看摘要与截图。
- `cancelled`、`timed_out`、`action_limit`、`failed`、`interrupted` 都不能作为任务成功。分别表示停止、超时、达到动作上限、执行失败及进程异常退出。
- 超时和取消会停止后续动作，无法撤销已送达桌面的操作。同一桌面不应有多个控制者；本地 CLI 文件锁只能约束同一锁目录及归一化 URL。
- Windows、macOS、Linux 的可用能力由实际 Cua 后端和桌面权限决定；Linux 不固定 Wayland，不补自定义驱动。不支持的操作保留为失败。
- 凭证通过配置引用文件，诊断与结果遮盖配置中的凭证；截图保留任务内容，按项目要求保管。
- Windows 服务安装器在写入前检查下载清单，在解包前检查全部归档成员；不接受越界路径、路径别名和链接。下载校验失败时保留已有文件。脚本需与附带的 `windows/` 文件一起使用，见 [运维说明](../../tools/computer-use/ops/README.md)。

安装、配置与方案调研见 [使用文档](../../tools/computer-use/README.md)，实现索引见 [computer-use 模块](../../agents/computer-use/index.md)。
