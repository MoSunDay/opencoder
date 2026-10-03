# Windows TUI 与 operator

支持 Windows 11 x64 原生运行，不需要 WSL。需要 PowerShell 7.4 及以上的 7.x 稳定版（`pwsh.exe`），推荐在 Windows Terminal 中使用。Windows PowerShell 5.1 不支持。Linux/macOS 的命令工具继续使用 `bash`；Windows 注册 `powershell`，模型收到的命令提示也随平台切换。

## 安装

先安装当前稳定版 PowerShell 7（最低 7.4）：

```powershell
winget install --id Microsoft.PowerShell --source winget
```

下载 Windows ZIP 与同名 `.sha256` 文件，核对 ZIP 的 SHA256 后解压。在 **PowerShell 7.4 及以上** 中执行：

```powershell
Get-FileHash .\opencoder-windows-x64-<commit>.zip -Algorithm SHA256
Expand-Archive .\opencoder-windows-x64-<commit>.zip .\opencoder-package
pwsh -NoProfile -ExecutionPolicy Bypass -File .\opencoder-package\install.ps1
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine')
opencoder --build-info
opencoder-agent --build-info
opencoder
```

默认安装到 `%LOCALAPPDATA%\OpenCoder\bin`，加入当前用户 PATH，无需管理员权限。新终端会读取更新后的 PATH；上面的命令也会刷新当前窗口的 PATH。可用 `-Destination 'D:\Apps\OpenCoder\bin'` 改目录，或用 `-NoPath` 跳过 PATH 修改。

更新前退出 TUI 和 operator 节点。安装器先检查包内校验和及两个程序的版本信息，在私有 `bin.install-state` 目录保存恢复记录，再替换整个 bin 目录；出错恢复旧目录，成功后保留 `bin.previous-*`。安装中断后重新安装会先恢复未完成的操作，也可显式执行：

```powershell
pwsh -NoProfile -File .\opencoder-package\install.ps1 -Recover
pwsh -NoProfile -File .\opencoder-package\install.ps1 -Rollback
```

自定义安装目录时两条命令都需传原来的 `-Destination`。`-Recover` 恢复未完成的安装；已完成时核验当前安装。`-Rollback` 回退最近一次安装，首次安装则移走本次程序并撤销本次加入的 PATH 项。恢复记录保留原始依据，失败后可重复执行；其他 PATH 项、配置和会话数据保留。程序运行时 Windows 会锁定 EXE，不支持 Linux 式原地热替换。

## operator 节点

连接现有 OpenCoder Server（Server 继续部署在 Linux/macOS），启动原生 Windows 节点：

```powershell
opencoder-agent --help
opencoder-agent --remote https://<server> --token <node-token> --workdir 'D:\OpenCoder\work'
```

节点只声明并接受 `operator` 任务。DAG、runc、Brain、Team、Todos、Project、Maintenance 和 Windows Server 部署不在支持范围内。任务拥有独立工作目录与配置快照；子进程的 HOME、USERPROFILE、APPDATA、LOCALAPPDATA 指向该任务的目录。取消、超时、节点退出会清理该任务的整个进程树；长命令可以转入后台，通过 `/ps` 查看、`/stop` 终止。

TUI 支持首次配置、会话恢复、模型切换、剪贴板图片和 `--wrap codex`；后者需要原生 `codex.exe` 在 PATH 上或配置其路径。`ts` 的 tmux 会话、Unix shell 脚本安装器和 Unix 工具安装不适用于 Windows。只读模式使用 PowerShell AST 检查并执行受控命令，动态调用、脚本块、重定向和未确认只读的选项会被拒绝。Git 外部 diff、textconv、fsmonitor 和继承的环境覆盖被禁用；配置了内容过滤器或部分克隆的仓库会明确拒绝只读 Git 查询。`rg` 忽略外部配置，不允许预处理程序或压缩解码程序。原生命令使用标准参数传递，保留引号和中文参数。

## 构建与验证

安装 Visual Studio Build Tools 的 C++ 工具链、Rust MSVC x64、CMake 和 LLVM。在干净的 Git checkout 中执行：

```powershell
pwsh -NoProfile -File scripts/platform/release/build-windows.ps1
```

生成 ZIP、SHA256 文件、包内 manifest 和校验和。CI 使用 Windows 原生 MSVC 编译、运行平台测试并验证安装器。人工验收还需在 Windows 11 与 Windows Terminal 中检查首次配置、输入/粘贴/窗口缩放、异常退出后的终端恢复、图片输入、真实模型流式输出和本地 Codex 会话。
