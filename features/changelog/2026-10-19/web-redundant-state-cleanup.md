Commit: 9a8213e3dba80353fefcf7d20aa57bf7461f27c8

# 清理 Web 冗余参数与配置副本

## 变化

- SSE 订阅不再计算、传递未使用的 `created` 标记，移除对应警告豁免。订阅计数、生命周期锁、空闲回收和 drain 保护不变。
- 删除仅供测试读取的主题变量副本及辅助转换；测试直接核对 CSS 与 antd token，保留颜色、字体、RGB 和引用检查。扩大扫描后发现两处未定义的样式变量，改为原有默认颜色。
- 删除 DAG 编辑器未调用的序列化函数及未使用参数；Agent 引用字段只保留名称，每项引用快照只解析一次。
- 节点可用性直接检查节点，和下拉选项共用禁用条件，省去临时选项数组与名称查找。下拉选项内容、节点准入条件和公共接口不变。
- 相对本轮开始的工作树，源码与测试净减少 104 行，其中生产源码 93 行、测试 11 行；保留此前改动。

## 功能与测试

| 保留的行为 | 定向测试 |
| --- | --- |
| 最后订阅者回收、执行中保留、锁等待、计数不下溢 | [handle_tests.rs](../../../crates/web/src/handle_tests.rs) 的 `release_subscriber_*` 与 `session_handle_evicted_when_creator_leaves_first` |
| drain 失败时保留订阅，回放及直播衔接与去重 | [handle_resume_failure_keeps_subscribers.rs](../../../crates/web/tests/handle_resume_failure_keeps_subscribers.rs)、[sse_overlap_dedup.rs](../../../crates/web/tests/sse_overlap_dedup.rs)、[sse_presubscribe_gap.rs](../../../crates/web/tests/sse_presubscribe_gap.rs) |
| CSS 与 antd 一致，无缺失变量，字体不变 | [theme.test.js](../../../crates/web/spa/src/theme.test.js)、[mono.test.js](../../../crates/web/spa/src/ui/mono.test.js) |
| Agent 引用内容及空值展示、节点选择与会话创建 | [agentDetail.dom.test.jsx](../../../crates/web/spa/src/agentDetail.dom.test.jsx) 的 `renders the Meta reference snapshot`；[fleet.dom.test.jsx](../../../crates/web/spa/src/fleet/fleet.dom.test.jsx) 的节点准入用例；`chat/nodeSelection.dom.test.jsx`、`chat/chatMode.dom.test.jsx` |
| DAG 校验及 JSON/画布切换、TODO 执行链接 | `dag/specValidate.test.js`、`dag/dag.dom.test.jsx`、`dag/editor/editor.dom.test.jsx`；[todosTab.dom.test.jsx](../../../crates/web/spa/src/project/todosTab.dom.test.jsx) 的 `displays execution type, name and ID resolved from the index` |

## 验证

- SPA 首次定向运行 10 个文件：153 项通过、2 项失败。修正字体换行比较和未定义样式变量后，主题文件 27 项通过；补测 TODO 执行链接 1 项通过。最终覆盖 11 个文件、156 项用例，包含 `agentsItems.test.js`。
- Chromium 对比两处样式修改前后的全部 475 项计算属性，结果一致；背景仍为 `#fafcff`。共享 CSS 声明和 antd 配置值不变。
- Rust 使用 `cargo test --offline --locked -p opencoder-web`：`--lib release_subscriber` 5 项、`--lib session_handle_evicted_when_creator_leaves_first` 1 项，以及上表三个独立集成目标 9 项全部通过。
- `cargo clippy --offline --locked -p opencoder-web --all-targets -- -D warnings` 零警告；修改 Rust 文件的 `rustfmt --check --edition 2021` 通过。
- `npm run build` 与 `scripts/check-spa-drift.sh` 通过；102 个本地文档链接有效，`git diff --check` 通过。构建仅报告已有的包体积提示。
- 未运行全量测试，未发布或操作已有数据库及鉴权数据。
