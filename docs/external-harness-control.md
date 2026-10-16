# 外部 Harness 的运行中输入

实现本协议的启动脚本可在 profile 的既有 `envs` 中设置 `OPENCODER_EXTERNAL_INPUT_PROTOCOL=1`。未启用的 Codex 会话保持原有中断再续接行为。此配置随会话冻结，只对采用新配置的新执行生效。

启用后，运行中的文本 steer 通过独立的受管子进程调用同一启动脚本，主观察进程继续运行。queue 在当前轮次结束后按原流程进入下一轮。控制调用仍在原执行的容器和工作目录中，不改变资源、账号或进程归属。

`OPENCODER_HARNESS_CONTEXT.control` 包含 `schema_version:1`、`action`（`steer` 或 `cancel`）、稳定 `input_id` 和原 `parent_input_id`。标准输入是完整请求 JSON，包含这些字段和 `prompt`、`original`、`images`、`seq`。正文通过 stdin 传输，避免环境变量大小限制。普通调用没有 `control`，stdin 和 stdout 沿用原提示词与 Codex JSONL 协议。

控制调用必须只输出一行 JSON，再以 0 退出：

```json
{"input_id":"external-input-123","status":"accepted","message":"","evidence":{"command_id":"original-control-command"}}
```

- `accepted`：已核对外部系统接受了这个稳定输入，绑定原目标轮次；取消时表示原远端执行已终止。
- `rejected`：明确未送达，说明原因。平台保留原输入并展示拒绝原因，不能自动变成下一轮提示词。
- `pending`：结果不确定。平台保留原输入身份再查询；不能换身份、新建任务或改成 followup。

外部实现负责按原会话、原输入和幂等键核对外部回执，不能把命令执行成功当作模型已经采用指令。平台在冻结资源目录的 `external-inputs/` 保存请求和回执；崩溃恢复先核对它们，不把已转发输入再次作为普通提示词提交。轮次完成时仍需处理本轮已接收的控制输入。

原生控制命令（如 `/plan`）需要明确的轮次边界，运行中的此类输入会显示未送达；不把控制命令传给外部模型。图片是否支持由外部实现明确接受或拒绝。技能和路径按既有会话规则解析后传入文本。

每次控制调用观察最多 20 秒，失去回执时只终止该控制进程，保留主观察器并重新核对同一输入。主观察器的空闲超时不会因控制轮询而延长。硬取消先尝试对原输入发出外部取消；超过取消观察窗口仍不确定时明确报告，再执行既有本地进程树清理，不能宣称远端已经停止。

实现：[control](../crates/session/src/harness/codex/control/mod.rs)；行为测试：[harness_control.rs](../crates/session/tests/harness_control.rs)。
