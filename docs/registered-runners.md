# 注册业务 Runner

Runner 是 DAG 的一种步骤。Server 管理命名 Codex 配置和运行入口，Node 负责持久化受理、排队、进程、取消和超时。业务程序保留原有取证、校验及报告流程。

## 配置

- `GET /api/harnesses/codex/profiles`、`PUT /api/harnesses/codex/profiles/:name`：私有 Codex 配置版本，包含 executable、model、reasoning_effort、sandbox_mode、approval_policy、auth_slot、envs。
- Agent 卡片设置 `harness: "codex"` 和 `harness_profile: "business"`；未绑定命名配置的 Agent 继续使用默认 Codex 配置。
- `GET /api/runners`、`PUT /api/runners/:name`：登记 command 数组、workdir、envs、files（绝对路径到 SHA-256）和可选 parent_unit。
- Web 的「Agent 配置」提供 Agent Harness、Harness 管理、Runner 管理页签；DAG 编辑器可选择 Runner 步骤。

```json
{
  "name": "eval-diagnose",
  "steps": [{
    "name": "diagnose",
    "kind": {"type": "runner", "runner": "eval-diagnose", "agent": "eval-diagnose"},
    "timeout_secs": 3600
  }]
}
```

受理时固定 Agent 的 NFS 资源版本、命名配置及 Runner 配置。之后更新配置不会改变排队任务。安装文件在受理和执行前校验；应使用不可变发布目录，保留仍被执行引用的旧版本。凭据文件和环境值保存在私有运行数据中，不提交 Git 或发布到 NFS。

## 前台协议

Runner 从 stdin 读取一行 `opencoder.runner.v1` 调用，包含 execution_id、step、input、context、output_dir、timeout_secs、unit、agent、codex 和版本号。agent 含已物化的 prompt 和 resources 路径，codex 为该 Agent 固定的参数。stdin 保持打开，取消消息为 `{"type":"cancel"}`。

stdout 仅允许 NDJSON，stderr 用于诊断：

```jsonl
{"type":"stage","stage":"collecting","detail":{"job_id":"example","attempt":1}}
{"type":"codex","phase":"analysis","event":{"type":"thread.started","thread_id":"example"}}
```

Codex 后续 JSONL 原样放入 `event`。每个 phase 必须收到完整 thread/turn 结束事件；现有解码器将思考、工具、回复写入 DAG 父会话，刷新后可重放。结果必须在所有 Codex 阶段完成后提交：

```json
{"type":"result","result_file":"result.json","artifacts":[{"file":"result.json","sha256":"<64 位 SHA-256>"}]}
```

文件均位于 output_dir，禁止路径逃逸、符号链接、重复文件及校验不符。退出码为 0、结果协议完整且所有文件校验成功，步骤才记为 done。业务 `pass / block / inconclusive` 与执行状态独立。失败可发送 `{"type":"error","error":"原因"}` 并非零退出。

## 生命周期与恢复

配置 parent_unit 时，Node 为 Runner 建立绑定节点服务的 systemd 临时单元。业务程序创建的隔离单元应绑定调用中的 unit，从而在取消、超时和节点退出时收回后代进程。

已校验完成的 Runner 可以重放完成收据；DAG 恢复时重新校验产物。只有 started 标记、没有完成收据的尝试不会隐式重跑，应通过业务任务重试接口创建新 attempt。Node 的最大并发数与 FIFO/LIFO 对 Runner 和其他执行统一生效。

执行详情提供 runners（阶段、资源和配置版本、摘要、准出结论、文件清单）；`/api/executions/:id/messages` 可重放 DAG 消息，artifact 接口支持 `step=diagnose&file=artifacts/<relative>`。业务适配器可通过 `annotate` 命令回写独立的报告投递状态。

协议版本为 7；Server 与 Node 一起升级。
