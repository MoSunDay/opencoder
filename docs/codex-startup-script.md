# Codex 启动脚本

Codex Harness 的默认配置和命名 profile 支持 `startup_script`，值为外部脚本的命令数组。OpenCoder 负责传递命令、输入输出、退出状态和取消信号。

```json
{
  "model": "your-model",
  "startup_script": ["/bin/sh", "/opt/scripts/v1/codex-start.sh"]
}
```

在「Agent 配置 → Harness 管理」编辑启动脚本，每行对应数组的一项。首项是脚本或解释器，后续项为参数；留空则直接启动 Codex。也可通过 `PUT /api/harnesses/codex` 或 `PUT /api/harnesses/codex/profiles/:name` 保存。

## 执行约定

- 实际执行为 `startup_script... <Codex 可执行文件> <Codex 参数...>`，参数逐项传递，不做 shell 拼接或插值。脚本必须负责执行追加的命令，并保留退出码。
- 脚本和 Codex 使用同一个工作目录、输入输出管道和进程管理范围。标准输入是任务提示词，应留给 Codex；标准输出只用于 Codex JSONL，脚本日志写到标准错误。
- 环境变量 `OPENCODER_HARNESS_CONTEXT` 是 JSON，包含 `schema_version: 1`、`session_id`、`agent`、`thread_id`、`fork_from`、`input_id` 和 `workdir`。新会话没有 thread ID；后续轮次带原 thread ID。它不包含账号或凭据。
- 每轮实际启动 Codex 时执行脚本，包括继续会话和分叉。脚本入口随 Harness 配置固定；外部脚本文件应使用可追溯的版本路径。
- 脚本可设置环境变量、准备私有文件或增加 Codex 参数；这些动态值不会写回 profile 或会话配置。
- 启动脚本缺失、退出失败或输出不合法时，本轮失败，不回退到直接运行 Codex。准备阶段也受既有流超时与任务取消约束。
- 取消会终止脚本和子进程。强制终止不能保证脚本清理代码运行，外部资源服务必须能够核对进程状态并回收占用。
- 容器任务在原有容器内运行脚本，解释器或可执行脚本必须存在于该容器镜像；脚本参数引用的文件同样必须在容器内可读，不回退到宿主机。
- 不再提供平台专用的 `auth_slot` 字段。旧配置中如带有该字段（包括 `null`），切换版本前须移除；需要扩展 Codex 参数时，由启动脚本自行组织。

## 外部脚本示例

以下文件由使用方部署在平台仓库外：

```sh
#!/bin/sh
set -eu
# 在这里读取外部配置、准备环境。
export CUSTOM_SETTING='configured by launcher'
exec "$@"
```

需要在正常结束后清理时，可以用 `"$@"` 启动并等待，记录其退出码，执行清理后返回该退出码。跨平台脚本可显式配置解释器，例如 `["C:\\Python\\python.exe", "C:\\scripts\\codex_start.py"]`；OpenCoder 不隐式选择 shell。
