# Windows 部署与回退

本目录用于将固定版本的 Cua 服务部署到已有 Windows 用户桌面。首次验证节点为 win-11，使用原生后端；不重启虚拟机，不建立公共 NodePort，不更换 OpenCoder 主模型。

## 安装服务

必须有已登录的交互式桌面，并先确认节点没有其他任务占用。`install-windows.ps1` 使用管理员权限创建防火墙规则，服务本身使用当前用户的普通权限、交互式计划任务启动。

安装前准备一个仅在受控网络内可访问的制品目录：

- `python.tar.gz`：Windows x64 Python 3.12 的独立发行包；本次使用 Astral python-build-standalone 20260807 / Python 3.12.13。
- `uv.zip`：Windows x64 uv；本次使用 0.12.7。
- `cua_computer_server-0.3.46-py3-none-any.whl`：从 README 指定的 Cua 提交构建。
- `windows-requirements.txt`：对该 wheel 使用 `uv pip compile --python-version 3.12 --python-platform windows` 生成的依赖清单。
- `packages/`：按清单下载的 Windows cp312 或通用 wheel，包括上述服务 wheel。离线安装避免 Windows 上下载依赖时卡住。
- `manifest.json`：每个文件的相对路径及 SHA256，格式为 `[{"file":"uv.zip","sha256":"..."}, ...]`。

独立发行包从 [Python Build Standalone](https://github.com/astral-sh/python-build-standalone/releases) 和 [uv](https://github.com/astral-sh/uv/releases) 官方发布下载；制品目录及清单中不包含凭证。

在目标用户的桌面会话中运行：

```powershell
.\install-windows.ps1 -ArtifactBase 'http://private-artifact-host:18180' `
    -HostAddress '192.168.127.10' -Gateway '192.168.127.1'
```

默认目录为 `$env:LOCALAPPDATA\OpenCoder\computer`。安装脚本核对下载文件的哈希，建立独立 Python 环境，生成 `serve.ps1`、`install.log` 和 `install-result.json`。计划任务名称为 `OpenCoder Cua Computer`，服务绑定虚拟机私有网卡的 8000 端口；防火墙规则 `OpenCoder Cua computer server` 只允许指定网关来源。`installed` 表示已安装并请求启动，必须再通过 CLI `doctor` 确认可用。

通过 Node API 执行安装时，不能用普通后台 `Start-Process` 脱离 API 作业进程。应创建仅用于本次安装的交互式计划任务，执行完成后删除该安装任务，保留 Cua 服务任务。断点重试可复用已校验文件和 Python 环境；已存在的服务任务或防火墙规则需要先检查归属，脚本不会覆盖它们。

## 连接控制端

支持 SSH 时优先使用主 README 的本机隧道。现有 Kubernetes Windows 虚拟机没有 SSH 时，可用 `forward.py`：

```bash
python3 forward.py --namespace bits-fleet --selector app=bits-fleet-11 \
  --guest 192.168.127.10 --local-port 18000
```

控制端需要已有的 Kubernetes 访问权限。脚本要求标签匹配唯一运行 Pod，在 Pod 的回环地址创建带 PID 记录的 socat 转发，再通过只绑定控制端 `127.0.0.1` 的 `kubectl port-forward` 接入。关闭时只清理自己记录并核验过的进程。转发恢复仅恢复连接，不重放任何 GUI 任务。

持续使用时把该命令放入独立 systemd 服务。本次控制端服务为 `opencoder-computer-win11.service`，配置目标 URL 为 `http://127.0.0.1:18000`。私有网段、防火墙来源限制和 Kubernetes 认证共同保护该连接；不能把无认证的 Cua 服务直接开放到公网。

```bash
opencoder-computer doctor --target win-11
opencoder-computer doctor --target win-11 --check-model --timeout 120
```

OpenCoder 主模型使用已有代理环境时，访问内网模型网关应使用能直连该地址的进程环境。本次诊断发现既有 OpenCoder 版本的显式代理只排除回环地址，不能依赖 `NO_PROXY` 排除内网地址；测试时仅对启动的 OpenCoder 进程移除了代理环境变量，全局配置保持不变。

本次内网 GLM 网关能够通过单次只读检查，但在多轮任务中返回过混合工具格式及未达成的完成摘要。因此桌面模型改用现有 `glm` provider 的直连接口，仍请求 `glm-5.3-flash`，主模型配置不变。只读检查通过后仍需做业务验收；最终摘要不能替代截图与保存文件的核对。

## 回退

先取消运行中的任务，并查询到终态。只撤销本次创建的条目：

1. 从 CLI 注册文件移除 `computer-use`；已有其他注册保留。移除独立 `computer.json` 和本次独有的密钥文件时先确认没有其他使用者。
2. 控制端停止并移除 `opencoder-computer-win11.service`；转发脚本会清理自己的隧道与 Pod socat 进程。
3. Windows 停止并删除 `OpenCoder Cua Computer`，删除对应防火墙规则。确认 Python 进程已退出后才删除本次服务目录。
4. `uv tool uninstall opencoder-computer` 卸载本次独立 CLI。回退前保存需要的运行证据。

回退不会自动撤销已经执行的桌面操作，也不会清理其他任务、桌面登录状态或鉴权数据库。
