Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# brain 模块

注册能力目录、不可变 v2 计划与统一图执行内核。计划以 input、实例、output、路由组织；能力自身负责执行和验证，大脑只消费统一输出契约。

## 执行边界

- `graph/validate.rs` 校验端口、固定输入映射、邻接目标、出口及可达性；手写和动态生成计划共用该入口。运行版本不能修改。
- `graph/advance.rs`、`routing.rs`、`causal.rs` 以纯函数推进激活分支、局部路由与因果汇合。每次回流生成独立 visit，输入引用固定到具体 output 轮次；未选择分支不参与等待。
- `graph/outputs.rs` 保存实际内容、产物引用及独立的完成／验证依据。未知判断不能满足相应出口要求；根完成还要求全部激活分支收敛。
- `activation.rs` 分开全图生成和局部路由模型请求。路由只序列化连接输出、路由语义、相邻输入描述及出口要求，不发送根目标或完整计划。
- `execution/` 管理激活版本、控制 epoch、幂等回执、暂停取消与资源请求。先构造 Prepared 回执，再由节点持久化、派发；后继请求含映射内容和同轮次 `source_outputs`。
- 非法路由、缺少输出、访问上限及能力契约错误进入 blocked，不创建回退能力。每实例默认最多访问 20 次。
- 历史决策树和 Playbook 类型保留供只读查询，旧写入／调度入口统一报迁移错误；没有第二套执行内核。

## 索引
- `crates/core/src/brain/` — v2 计划、图状态、输出与路由 DTO
- `crates/brain/tests/` — action_flow/execution/planning/ontology 行为回归

## 相关
- [features/brain](../../features/brain/index.md) — 工作台操作面
- [agents/control](../control/index.md) — 派发 API
- [agents/worker](../worker/index.md) — 持久化、能力输出适配与恢复
- [运行协议](../../docs/brain-orchestration.md) — 输入文档、输出、路由和迁移契约
