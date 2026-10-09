# OpenCoder computer use

`opencoder-computer` 是可选 CLI。OpenCoder 通过现有 `/cli` 注册使用说明，再用 bash 启动任务。Cua 的 `ComputerAgent` 负责看截图、调用桌面模型、执行动作和检查结果；桌面模型与 OpenCoder 的模型分别配置。

实现使用 Cua 原生 Computer Server，不创建虚拟机，不接管远端服务的生命周期。Windows、macOS、Linux 的实际能力取决于远端 Cua 后端及其权限，Linux 不限定 Wayland，也没有自研 Wayland/X11 驱动。

## 方案调研与版本

调研日期：2026-10-01。选型依据是操作已有远程桌面、独立 Agent 完成自然语言任务、可通过 CLI 接入。

| 方案 | 与本次需求的关系 | 结论 |
| --- | --- | --- |
| [Cua](https://github.com/trycua/cua) | Computer SDK、Agent SDK、Computer Server 和原生 Driver；可连接桌面并运行模型循环 | 本次采用 |
| [UI-TARS Desktop](https://github.com/bytedance/UI-TARS-desktop) | 提供桌面 Agent 和 SDK，适合视觉操作；本次接入会需要另一套执行组件 | 保留为后续替代方案 |
| [Agent S](https://github.com/simular-ai/Agent-S) | 桌面任务规划与操作框架，需要配置推理和定位模型 | 本次先不引入第二套 Agent |

锁定 Cua 源码提交 [`9545a3d17b44b587b593d59090dc140740876a6e`](https://github.com/trycua/cua/tree/9545a3d17b44b587b593d59090dc140740876a6e)，对应 Agent SDK 0.8.4、Computer SDK 0.5.19、Computer Server 0.3.46。使用源码依赖是为了包含发布包之后的上游修复；升级时应一起更新依赖、远端服务及 SDK 集成测试。

当前仓库还发布了较新的 Rust Driver，但上述 Computer Server 的可选 `driver` 依赖要求 `cua-driver >=0.22.2,<0.23.0`。启用 Driver 时让上游依赖解析选择匹配版本，不能把最新 Driver 的平台能力直接当成该服务的能力。平台限制以 [Cua 平台文档](https://cua.ai/docs/reference/cua-driver/platform-support)及实际安装版本为准。

## 安装 CLI

需要 Git、[uv](https://docs.astral.sh/uv/) 和 Python 3.12 或 3.13。在 OpenCoder 仓库根目录执行：

```bash
uv tool install --python 3.12 --torch-backend cpu ./tools/computer-use
opencoder-computer --help
```

本次接入范围是本机 OpenCoder CLI/TUI。CLI、Python 依赖、配置和凭证文件需要在其 bash 工具实际执行的环境中可访问。远端 DAG/runc 执行节点的安装和配置分发不在本次范围内。

这不会修改 Rust 运行时或默认安装流程。开发时可以使用独立环境：

```bash
uv venv --python 3.12 /tmp/opencoder-computer-dev
uv pip install --torch-backend cpu --python /tmp/opencoder-computer-dev/bin/python -e './tools/computer-use[test]'
/tmp/opencoder-computer-dev/bin/pytest tools/computer-use/tests -q
/tmp/opencoder-computer-dev/bin/ruff check tools/computer-use
```

## 准备远端桌面

在已登录的目标桌面内安装相同提交的 Computer Server。macOS/Linux 示例：

```bash
uv venv --python 3.12 ~/.cua-server
uv pip install --python ~/.cua-server/bin/python \
  'cua-computer-server @ git+https://github.com/trycua/cua.git@9545a3d17b44b587b593d59090dc140740876a6e#subdirectory=libs/python/computer-server'
~/.cua-server/bin/python -m computer_server --host 127.0.0.1 --port 8000
```

Windows PowerShell 使用 `uv venv --python 3.12 "$HOME\.cua-server"`，安装时 Python 路径改为 `"$HOME\.cua-server\Scripts\python.exe"`，然后用该 Python 执行 `-m computer_server --host 127.0.0.1 --port 8000`。必须运行在要操作的用户桌面会话里。macOS 按 Cua 要求授予辅助功能和屏幕录制权限；Linux 默认服务依赖上游桌面后端，能否使用 Wayland 由该后端决定。

若要使用服务已有的 Driver 后端，安装依赖时将包名改成 `cua-computer-server[driver]`，启动时追加 `--backend cua-driver`。不额外启动新版本 Driver，也不绕过上游的不支持操作。

推荐保持服务监听本机，通过 SSH 隧道接入。例如在 OpenCoder 所在机器执行：

```bash
ssh -N -L 18000:127.0.0.1:8000 user@desktop-host
```

这时目标 URL 是 `http://127.0.0.1:18000`。也可以使用受保护的 HTTPS 反向代理；代理必须支持 `/cmd` 和 `/ws`，URL 可以包含路径前缀。需要认证时给目标增加 `headers_file`，指向 JSON 请求头文件。不要在 URL 中放密码或 token。

## 配置与命令

参考 [computer.json](examples/computer.json)，保存到 `~/.opencoder/computer.json`。可以只保留一个目标；`os` 必须是 `windows`、`macos` 或 `linux`。`model.name` 使用 Cua 支持的模型名称，`model.api_base` 可指定模型端点。该示例采用 Cua 的 Anthropic 原生循环。

GLM-5.3-flash 使用 [computer-glm.json](examples/computer-glm.json)。从现有 OpenCoder provider 配置读取模型接口地址，把凭证写入独立密钥文件；不要修改主模型配置。`native_tool_calls: true` 让 Cua 的通用视觉循环向接口发送原生函数定义，并通过 Cua 回调明确归一化坐标。动作解析、坐标还原和执行仍由 Cua 完成。安装已包含这一循环的 Qwen 与 CPU 图像处理依赖，首次安装体积较大。其他模型默认不启用该选项。

原生函数模式在执行前校验模型响应：混合工具文本、额外参数、并行动作和非法坐标都会使任务失败。多轮预测补充当前截图，使用 Cua 自带的图像保留回调减少旧图干扰。Windows 原生滚动参数是滚轮刻度，向模型说明使用小幅度滚动或 `Ctrl+End`，避免按屏幕像素传入大数。Windows 文本输入使用 Cua 的 `set_clipboard` 与 `hotkey`，解决已测布局下直接按键输入损坏汉字的问题；这会替换目标桌面的剪贴板内容。

`model.api_key_file` 是纯文本密钥文件；`targets.<name>.headers_file` 是请求头 JSON 文件。这两种路径都相对于配置文件目录解析。凭证不放进任务、CLI 参数或 Git；Unix 下建议把配置和密钥文件权限设成 `600`。

```bash
opencoder-computer doctor --target linux
opencoder-computer doctor --target windows --check-model --timeout 120
opencoder-computer run --target linux --task-file task.txt \
  --output-dir .opencoder/computer/runs/demo --timeout 600 --max-actions 50
opencoder-computer status --run-dir .opencoder/computer/runs/demo
opencoder-computer cancel --run-dir .opencoder/computer/runs/demo
```

自定义配置用 `opencoder-computer --config PATH run ...`，配置参数放在子命令前。`--task-file -` 可以从 stdin 读取任务。输出目录必须是新目录；同一 URL 的任务在本地通过系统文件锁排他执行，进程退出会释放锁。不同机器或不同 URL 指向同一桌面时仍需由调用方避免并发。

任务文件要描述具体目标和验收条件，例如：“打开计算器，算出 17 × 23，确认显示 391，并在最终回答中报告实际看到的结果。”

`doctor` 检查 SDK 加载、模型循环选择、桌面连接、截图解码、屏幕尺寸及服务版本，并尽量读取桌面环境。默认不会调用模型 API。`--check-model` 会请求模型描述当前截图，并在启用原生工具调用时预测合成图片上的点击，验证函数格式及坐标转换；它会调用模型 API，但不执行桌面动作。该检查不能证明所有输入权限或任务都可用。`run` 才执行模型任务。运行中可用 `status` 查询，取消请求之后继续查询直到任务终态。

每次命令 stdout 只输出一行 JSON，长摘要会缩短，完整结果保存在 `result_file`；诊断输出走 stderr 并遮盖配置中的凭证。截图以 PNG 文件保存，事件中不嵌入图片。`status`、`cancel` 不需要模型配置或连接远端桌面。

| 状态 | 退出码 | 含义 |
| --- | --- | --- |
| `ready` / `running` | 0 | 桌面检查通过 / 任务仍运行 |
| `completed` | 0 | Cua 正常返回最终助手结果；任务是否达成应检查摘要与截图 |
| `cancel_requested` | 0 | 已写入取消请求，尚未确认停止 |
| `cancelled` | 130 | 已停止继续执行 |
| `timed_out` | 124 | 超过任务时限 |
| `action_limit` | 1 | 达到动作上限，未执行下一动作 |
| `failed` / `interrupted` | 1 | 模型、连接、能力或产物写入失败 / 进程退出却没留下最终结果 |

动作上限包含模型主动请求的截图与等待等操作，初始/最终留档截图不计入。`cancel`、SIGINT、SIGTERM 会停止后续动作并清理连接，无法撤销已发送给桌面的动作。截图和摘要可能包含任务内容，应按项目数据要求保管运行目录。

## 接入 OpenCoder

在 TUI 中用 `/cli` 新建 `computer-use`，启用并选择 `parent`，内容使用 [cli.json](examples/cli.json) 中的 `content`。也可把该条目合并到项目 `.opencoder/cli.json` 或全局 `~/.opencoder/cli.json`；保留已有注册，文件根对象就是注册名称到配置的映射。

OpenCoder 将任务写入文件，通过 bash 前台启动上面的 `run` 命令，不使用 shell `&`、`nohup` 或 `setsid`。命令超过 bash 的前台等待期限时，由 OpenCoder 自动移交后台管理；保存返回的进程句柄，查询已知运行目录，不重复启动任务。不要预建运行目录，也不要在启动前把命令输出重定向到运行目录内。需要查看截图时，使用已有 `view_image` 工具打开 `screenshots` 目录中的实际 PNG 文件。

## 验证范围

自动化测试使用真正安装的 Cua SDK、真实 CLI 子进程及本地 HTTP/WebSocket 服务桩，覆盖模型循环、认证头、路径前缀、动作拒绝、截图产物、取消、超时、动作上限、凭证遮盖和异常退出。Rust 测试 `computer_cli_registration` 使用真实 OpenCoder 配置加载器验证注册及注入范围。

Windows 部署、受保护的转发及回退步骤见 [Windows 运维说明](ops/README.md)。实际桌面的验收必须检查截图及任务产物；服务桩测试不能替代真机验收。macOS/Linux 的原生后端尚未在本次环境验证。
