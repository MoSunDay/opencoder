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

当前 `dag.agent_sandbox = "runc"` 路径尚未接入 Codex 可执行文件、配置和宿主登录态，不能用于上述默认凭证方案。Codex DAG 应使用默认的 `host` 路径。

使用父 Agent 调度独立任务时，使用 [TODO 目录工作台](todo-workbench.md)。
