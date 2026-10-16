# DAG 步骤执行类型

DAG 步骤支持 `agent` 和 `binary`。注册 Runner 的配置、执行、查询和管理接口已移除，旧 Runner 步骤定义会在校验时被拒绝。

Codex 使用 `agent` 步骤接入：将目标 Agent 的 `meta.json` 设置为 `"harness": "codex"`，在 DAG 中引用该 Agent，例如：

```json
{
  "name": "codex-check",
  "steps": [
    {
      "name": "check",
      "kind": {
        "type": "agent",
        "agent": "codex-review",
        "prompt": "检查输入，并在最终回复末尾用 JSON 围栏输出结果。"
      }
    }
  ]
}
```

所有 DAG 步骤都在同一个运行级 `runc` 容器内执行。Codex 使用实际执行节点用户的登录态：设置了 `CODEX_HOME` 时使用该目录，否则使用 `~/.codex`。节点需已安装 Codex 并完成登录；纯 Codex DAG 不要求 OpenCoder 原生模型的 API Key。Server 与执行节点分离时，凭证取自执行节点，Server 不会自动分发自身的登录文件。

显式配置的 Agent `harness_profile` 或全局 Codex Harness 设置仍可覆盖执行环境，例如 `envs.CODEX_HOME`；要使用节点默认登录态，应省略这些凭证目录覆盖。认证失败会使步骤及 DAG 报错，依赖步骤不会继续执行，也不会改用原生模型。

共享 DAG 容器支持 Codex，静态步骤和动态实例沿用同一 Harness/profile 选择、依赖输出与事件查询接口。节点需先准备包含原生 Codex CLI 的 rootfs，再启用容器执行：

```bash
scripts/prepare-dag-rootfs.sh /path/to/node-data/dag/rootfs --codex /path/to/native-codex
```

```json
{"dag":{"rootfs_dir":"/path/to/node-data/dag/rootfs","workspace_dir":"/mnt/dag-workspace","binary_dir":"/mnt/dag-binary"}}
```

安装参数接受 Linux 原生 ELF 可执行文件；npm、Python 或 Shell 启动器应改为提供其实际调用的原生 Codex 二进制。脚本同时安装 Shell、Git、TLS 证书、动态库及旧版 glibc 所需的 NSS 域名解析模块，并复制节点的 hosts 配置。默认在容器中执行 `/usr/bin/codex`；profile 的 `executable` 如有设置，表示 rootfs 内的绝对路径，需在对应位置安装文件及依赖。节点准入检查 rootfs、Codex 可执行文件和凭证目录，缺失时直接拒绝；不会转到宿主执行。

容器直接挂载节点原有的 Codex 目录，并将容器 `CODEX_HOME` 指向该挂载。目录保留可写，以支持 Codex 的原子认证刷新、锁文件和会话存储；不复制或另外生成认证文件。节点默认目录与 profile 指定目录都必须是有效的绝对路径。Harness 私有配置独立于 DAG 产物保存；模型、认证槽位、推理强度及显式环境设置会传给容器中的 Codex。步骤显式 `model` 优先于 profile 模型。代理环境沿用节点的常用 HTTP/SOCKS 代理设置，其他运行依赖和配置引用的容器外绝对路径需一并准备到 rootfs。

外层 OCI rootfs、知识库和 Agent 资源池保持只读，步骤目录可写。Codex 内层默认使用 `sandbox_mode="danger-full-access"`、`approval_policy="never"`，由外层 `runc` 执行隔离；显式 profile 策略仍优先。取消和超时只终止当前步骤进程树；整次运行结束才关闭共享容器，认证失败和异常事件流使步骤失败。

注册 Agent 的职责、skills、prompts 与 Harness 配置分别注册；`act` 只是 OpenCoder harness 内置模式。使用管理账号占用的 Codex wrapper 时，将七个源码文件按内容摘要安装到部署根的 `services/codex-launchers/<摘要>`，并显式为该 Agent 设置 `startup_script=["<目录>/run","--config","<私有配置路径>"]`。启动脚本接收平台已有 session，不创建另一个 session。

```bash
scripts/prepare-dag-rootfs.sh /path/to/image --codex /absolute/native/codex \
  --codex-startup /absolute/immutable/launcher-directory
```

镜像包含同版 `run`、`accounts.py`、`leases.py`、`policy.py`、`state.py`、`client.py`、`broker.py` 和真实 Codext，保存逐文件摘要。Runtime 检查宿主与镜像内容一致后冻结配置；配置只引用既有 `deployment_config`，从唯一 `deployment.state_dir` 派生 `runners/codex`，不重复配置存储根。私有代理、部署声明、`client_config_home/opencoder/ctl.json`、`connection_files` 显式声明的连接及其确切 token 文件只读挂载；原 Codex home 保留可写，不复制鉴权文件。带此声明的发布镜像同时固定该版 `opencoder-cli`。

容器 wrapper 通过每个步骤的私有 Unix socket 让宿主登记账号占用，共用已有锁与每账号并发限制；不把容器 PID 写入宿主账本。wrapper 退出只报告结果，Runtime 完成步骤进程树清理后才释放。Runtime 异常断开时 broker 保留占用到整个对应容器消失。该 broker 只管理账号选择和占用，不在宿主执行 Agent。

使用父 Agent 调度独立任务时，使用 [TODO 目录工作台](todo-workbench.md)。

额外连接由启动配置中的可选 `connection_files` 数组声明，每项为已有 JSON 文件的绝对路径。例如：

```json
{"connection_files":["/srv/runner-config/connections/service.json"]}
```

省略或传空数组时只挂载平台 CLI 连接；运行时不扫描配置目录，也不按外部工具名称寻找文件。连接中的 `token_file` 可以是绝对路径，或相对该连接文件所在目录的路径；只挂载该文件和它明确引用的 token 文件。声明文件缺失、格式错误、路径非绝对或包含软链接时拒绝启动。既有配置需要在下一次启动前显式列出额外连接；已冻结的运行保持原挂载。
