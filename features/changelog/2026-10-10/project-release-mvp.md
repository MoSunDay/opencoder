Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# 项目人员字段与完成前提校验

项目、专项、TODO 新增独立负责人和关注人，界面、API 和 CLI 使用相同字段。人员关系复用现有用户身份；非法人员和无效标签不能留下部分修改，删除用户或对象清理对应关系。

TODO 完成请求可携带读取时的任务、归属、能力、执行及依赖前提。Server 在同一事务中核验前提与看板修改，过期前提返回 409。原生执行结果仍从所属节点读取。

Codex 首轮与续接入口使用堆分配，避免普通 Tokio 工作线程栈溢出。

## 测试覆盖

| 功能 | 测试入口 |
| --- | --- |
| 人员独立、清空、迁移、权限与删除 | [project_people.rs](../../../crates/store/tests/project_people.rs)、[HTTP 测试](../../../crates/control/tests/e2e/project_api/people.rs)、[字段交互](../../../crates/web/spa/src/project/people/fields.dom.test.jsx) |
| 并发修改与完成前提 | [acceptance.rs](../../../crates/control/tests/e2e/project_api/acceptance.rs) |
| CLI 人员字段 | [parse_project_brain_agents.rs](../../../crates/ctl/tests/parse_project_brain_agents.rs) |
| 普通线程栈上的首轮与续接 | [harness_stack.rs](../../../crates/session/tests/harness_stack.rs) |

相关索引：[项目](../../project/index.md)、[Store](../../../agents/store/index.md)、[Session](../../../agents/session/index.md)。
