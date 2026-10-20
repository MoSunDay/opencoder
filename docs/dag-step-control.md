# DAG 单步恢复

步骤设置 `recovery_policy: "operator"` 后，执行错误进入 `waiting_recovery`，等待控制命令处理。省略该字段保留原来的失败行为。`timeout_secs` 省略或为 `null` 时没有步骤期限，外部巡检可根据实际进展判断异常。

执行节点须声明 `dag_step_control_v1`。先查询状态，再使用返回的 `attempt` 操作：

```sh
opencoder-cli dag steps status RUN_ID
opencoder-cli dag steps pause RUN_ID STEP --expected-attempt 1 --operation-id inspect-001 --reason '检查当前执行'
opencoder-cli dag steps status RUN_ID --operation-id inspect-001
opencoder-cli dag steps resume RUN_ID STEP --expected-attempt 1 --operation-id resume-001 --reason '原状态已核对'
```

动态步骤可加 `--instance 0`，只操作一个实例；省略实例编号控制整组未完成实例。`retry` 与 `resume` 都只接受已停止的未决尝试。`fail` 明确结束目标并保留原因，不提供强制成功命令。

命令绑定 `operation_id`、步骤、实例、预期尝试次数和原因。重复提交返回原回执；换内容复用标识或尝试次数已变化会拒绝。`queued` 表示已保存命令，暂停须等目标进程树回收后才返回 `applied`。

暂停和等待恢复不是终态，依赖步骤以及 `all_done` 收尾继续等待。其他分支可运行，已完成实例不重跑。重新开始前把上次产物保存到 `attempts/<attempt>`，使用原工作区和冻结输入。Agent 恢复原 session 的消息及输入消费记录，追加继续指令，不重新提交原始提示词。

节点重启后先用原运行的恢复入口重建执行环境；未决步骤保持等待，检查后再发单步命令。步骤进程退出只证明本节点执行器已结束；工具启动的外部任务仍须查询其原身份和结束回执。

真实 runc 用例覆盖目标隔离、局部重试和原 Agent session 续跑；独立 Worker 进程强杀测试覆盖同节点重建、保留已完成步骤及原 context 归档。发布与在线任务连续性另按[执行约定](../rules/04-dag-execution-contract.md)验收。
