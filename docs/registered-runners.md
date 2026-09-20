# DAG 步骤执行类型

DAG 步骤支持 `agent` 和 `wasm`。注册业务 Runner 的配置、执行、查询和管理接口已移除，旧 Runner 步骤定义会在校验时被拒绝。

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

默认 `dag.agent_sandbox = "host"` 时，Codex 子进程继承实际执行节点服务进程的环境，沿用该用户的 Codex 登录态：设置了 `CODEX_HOME` 时使用该目录，否则使用 `~/.codex`。节点需已安装 Codex 并完成登录；纯 Codex DAG 不要求 OpenCoder 原生模型的 API Key。Server 与执行节点分离时，凭证取自执行节点，Server 不会自动分发自身的登录文件。

显式配置的 Agent `harness_profile` 或全局 Codex Harness 设置仍可覆盖执行环境，例如 `envs.CODEX_HOME`；要使用节点默认登录态，应省略这些凭证目录覆盖。认证失败会使步骤及 DAG 报错，依赖步骤不会继续执行，也不会改用原生模型。

`dag.agent_sandbox = "runc"` 同样支持 Codex，静态步骤和动态实例沿用同一 Harness/profile 选择、依赖输出与事件查询接口。节点需先准备包含原生 Codex CLI 的 rootfs，再启用容器执行：

```bash
scripts/prepare-dag-rootfs.sh /path/to/node-data/dag/rootfs --codex /path/to/native-codex
```

```json
{"dag":{"agent_sandbox":"runc"}}
```

安装参数接受 Linux 原生 ELF 可执行文件；npm、Python 或 Shell 启动器应改为提供其实际调用的原生 Codex 二进制。脚本同时安装 Shell、Git、TLS 证书、动态库及旧版 glibc 所需的 NSS 域名解析模块，并复制节点的 hosts 配置。默认在容器中执行 `/usr/bin/codex`；profile 的 `executable` 如有设置，表示 rootfs 内的绝对路径，需在对应位置安装文件及依赖。节点准入检查 rootfs、Codex 可执行文件和凭证目录，缺失时直接拒绝；不会转到宿主执行。

容器直接挂载节点原有的 Codex 目录，并将容器 `CODEX_HOME` 指向该挂载。目录保留可写，以支持 Codex 的原子认证刷新、锁文件和会话存储；不复制或另外生成认证文件。节点默认目录与 profile 指定目录都必须是有效的绝对路径。Harness 私有配置独立于 DAG 产物保存；模型、认证槽位、推理强度及显式环境设置会传给容器中的 Codex。步骤显式 `model` 优先于 profile 模型。代理环境沿用节点的常用 HTTP/SOCKS 代理设置，其他运行依赖和配置引用的容器外绝对路径需一并准备到 rootfs。

外层 OCI rootfs、知识库和 Agent 资源池保持只读，步骤目录可写。Codex 内层默认使用 `sandbox_mode="danger-full-access"`、`approval_policy="never"`，由外层 `runc` 执行隔离；显式 profile 策略仍优先。取消和超时终止整个容器进程树，认证失败和异常事件流使步骤失败。

使用父 Agent 调度独立任务时，使用 [TODO 目录工作台](todo-workbench.md)。
