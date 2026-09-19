Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# 事件驱动的大脑调度 v3

## 背景

大脑过去保存完整计划、路由上下文和持续推进状态，无法把能力执行与调度判断解耦。v3 改为按轮次选择能力、等待节点终态事件、再依据执行索引判断下一步；子执行详情继续由所属节点维护。

## 变更

- **协议与纯函数**：新增 `schema_version: 3` 调度契约、能力目录归一化、输入绑定引用、严格 `Dispatch`/`Complete`/`Fail` 决策、轮次屏障和失败取消逻辑。非法能力、重复能力、缺少引用或无证据完成直接阻塞，不创建回退能力。
- **存储**：新增 scheduler run/operation/event 三类记录和 Store 接口。generation 冲突、终态事件去重、事件序号分页和终态冻结在事务内完成；事件只保存执行 ID、引用和摘要，不保存输入、消息、DAG 状态或产物正文。
- **Control 与 Worker**：v3 API 负责规则预筛、模型判断和真实能力派发；worker 通过 outbox 发送唤醒、派发、取消和终态确认，重启只恢复未确认事件与未完成创建请求。任一失败终态取消同轮兄弟操作，迟到事件不改写已终态脑状态。
- **查询与 CLI**：新增最小运行快照、事件页、轮次 operation 索引和 pause/resume/cancel；`brain runs` 增加轮次查询。v2 运行和路径保持兼容，不迁移或删除既有数据。

## 测试

- 纯函数：调度决策校验、输入引用、并行屏障、全部成功唤醒、单个失败取消兄弟、重复/乱序/迟到事件和轮次上限。
- Store：run/operation/event 原子提交、generation 栅栏、终态事件幂等、事件分页。
- Worker/回归：节点唤醒确认、执行输出索引、v2 brain e2e、workspace 全量测试。
- 验证命令：`cargo test --workspace`、`cargo check --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace`。
