Commit: 9a8213e3dba80353fefcf7d20aa57bf7461f27c8

# 清理重复逻辑与失效测试入口

## 变化

- 删除未调用的前端辅助函数、状态映射副本和导入；移除没有生产写入方的聊天节点预选状态，测试改为操作真实节点选择器。共享节点列表继续供 DAG 节点名称展示使用。
- 消息测试通过实际消费事件进入 reducer；仅乐观回显去重测试保留一个最小状态夹具。DAG 图测试直接使用步骤快照，重放、迟到事件和运行结束覆盖当前 `dag/run/model.js`。
- 执行详情直接使用地址中的执行 ID，删除重复选中状态；名称从当前列表读取，避免地址更新覆盖名称，浏览器返回无执行链接的页面时关闭详情。未出现在列表中的历史执行仍可直接打开。
- 删除已脱离模块树的 TUI 文件扫描器及 `ignore` 直接依赖；405 个 Cargo 依赖节点的版本和启用特性不变。5 个有效控制命令测试直接接回测试入口，移除空入口文件。
- 删除已脱离测试入口且要求“失败时取消兄弟执行”的旧 schema 3 测试；当前规则要求收齐整层结果，使用已有 schema 7 测试验证。
- Ontology 类型检查启用未用变量与参数检查；导航名称测试按页面拆为独立用例，避免多个真实页面共用单个超时额度。
- 相对本轮开始时的工作树，源码与测试净减少 646 行，其中生产源码 371 行、测试及夹具 275 行。保留此前工作树改动。

## 功能与测试

| 保留或修复的行为 | 定向测试 |
| --- | --- |
| 显式选节点后创建会话、切换节点与消息回显 | [nodeSelection.dom.test.jsx](../../../crates/web/spa/src/chat/nodeSelection.dom.test.jsx)、[sidebar.dom.test.jsx](../../../crates/web/spa/src/sidebar.dom.test.jsx)、[reduce.test.js](../../../crates/web/spa/src/reduce.test.js) |
| DAG 重放去重、迟到事件和动态进度 | [model.test.js](../../../crates/web/spa/src/dag/run/model.test.js) 的 `keeps terminal receipts over delayed or duplicate lifecycle events`、`applies run lifecycle and newest dynamic progress without changing other steps` |
| 执行名称、取消与恢复、链接及浏览器返回 | [team.dom.test.jsx](../../../crates/web/spa/src/team.dom.test.jsx) 的 `hits cancel then resume on the detail drawer action buttons`；[links.dom.test.jsx](../../../crates/web/spa/src/fleet/navigation/links.dom.test.jsx) 的历史导航用例，以及同目录 `session`、`records` 用例 |
| 复合控制命令保留技能参数；`@` 原样提交 | [forward_skill.rs](../../../crates/tui/src/control_helpers_tests/forward_skill.rs) 的 5 项用例；[key_handler_file_mention_tests.rs](../../../crates/tui/src/key_handler_file_mention_tests.rs) 的 2 项用例 |
| 失败后收齐同层结果且不自动重试 | [milestone.rs](../../../crates/brain/tests/milestone.rs) 的 `parallel_failures_wait_for_the_entire_frozen_dispatch_and_do_not_retry` |

## 验证

- SPA 初次定向运行 39 个文件：434 项通过、3 项失败；修正上述状态同步和测试选择器、拆分导航用例后，复跑失败文件及直接受影响的执行链接测试，6 个文件、51 项通过。新增的返回关闭断言单独通过。合并最终结果为 42 个文件、462 项通过。
- Rust 定向测试：`opencoder-tui --lib control_helpers::tests` 5 项、`key_handler::file_mention_tests` 2 项，`opencoder-brain --test milestone` 上述指定用例 1 项通过；均使用 `--offline --locked`。
- `cargo clippy --offline --locked -p opencoder-tui --all-targets -- -D warnings`、修改 Rust 文件的格式检查、`npm run typecheck:ontology` 通过；494 个前端源码及测试文件的相对导入有效。
- `npm run build` 通过，`scripts/check-spa-drift.sh` 确认产物一致；修改文档的 125 个本地链接有效，`git diff --check` 通过。未运行全量测试，未发布或操作已有数据库及鉴权数据。

## 保留事项

- [Bash guard 旧回归文件](../../../crates/session/src/bash_guard_bypass_regression.rs) 保留原样：审查发现包装命令识别缺口和过时断言，本轮不改变拦截行为或接入这些旧用例。
- [sandbox_inputs.rs](../../../tests/operator_e2e/sandbox_inputs.rs) 仍依赖已移除的旧测试接口，需要独立迁移，本轮保留原样。
