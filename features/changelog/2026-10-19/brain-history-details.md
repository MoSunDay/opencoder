Commit: c139569c708859863cadc589ce49a6dcaf371f13

# 大脑运行历史与详情阅读

大脑调度的“工作台”改为“运行历史”，每行表示一次执行，以执行时的计划名称为主、执行 ID 为辅。详情保留右侧抽屉与画布，默认先显示交付摘要和异常，避免将调度记录、输入和原始数据堆在同一页。

## 行为与兼容

- 列表使用服务端游标，每页 20 条；刷新保留当前页，从运行页返回恢复页码与滚动位置。计划名称从冻结 Assignment 批量提取，计划改名或删除不会改写历史，节点离线仍可浏览列表。缺失名称明确提示。
- 运行详情分为概览、执行记录、调度记录、输入与产物；轮次选择不随后台更新跳走。执行终态和层级达标分别显示，返工使历史达标结论失效时明确标记。
- 子执行在同一抽屉中按结果、执行过程、输入与产物、原始数据阅读，返回恢复父详情位置；并行层可返回“本层全部任务”并重新打开任一任务。基本信息在窄屏使用单列，避免节点编号与时间被挤压。共享执行详情的数据读取，不增加第二份详情请求。
- 长文可展开和复制，目标、摘要及调度说明支持安全 Markdown；结构数据按字段或表格展示，保留 `false`、`0`、`null`、空集合与未知字段。大字段沿用分段读取，文件下载保留来源执行与实例编号，支持服务路径前缀；明确关闭鉴权时使用浏览器直接流式下载，其余情况使用同源 Service Worker，凭据不进入 URL。页面清理地址参数或切换内部路由后，下载仍按发起页面身份和当前地址核验。
- 本次输入读取冻结请求的实际输入；原始数据按需展开。“补充信息”展开人工输入，提交失败保留草稿，托管子执行仍由大脑统一处理引导。
- DAG 过程图在状态刷新、切换页签后保留已测量的节点尺寸与连线位置；长错误信息可撑高卡片，避免偶发空白或连线偏移。
- 切换执行或关闭时取消未完成读取，迟到响应不能覆盖当前内容。历史 schema 4/5/6 保持只读。
- `/api/brain/runs` 增加可选 `name` 并使用 `limit`；返回结构和游标字段保留，五字段执行索引协议不变。无数据库迁移，不修改已有运行和认证数据。

## 功能与测试

| 功能 | 定向测试 |
| --- | --- |
| 冻结名称、无节点列表、缺失名称、游标及非法参数 | `brain_api::history::history_uses_frozen_names_and_cursor_without_a_live_node` |
| 名称批量投影与类型选择 | Store `execution_names` 3 项；Control `api::executions::paging` 4 项 |
| 翻页、刷新、读取失败、取消与迟到响应 | `history/history.dom.test.jsx` 2 项 |
| 历史评估失效、并行任务轮次归组 | `milestone/details/model.test.js` 2 项 |
| 人工输入、六类子执行、预算、详情导航与只读版本 | `milestone/run.dom.test.jsx` 19 项 |
| 页签滚动位置与后台更新保留历史选择 | `milestone/details/reading.dom.test.jsx` 2 项 |
| 子执行结果、单次读取、托管限制、取消及离线重试 | `fleet/detail/presentation/brain.dom.test.jsx` 3 项 |
| 结构化值、Markdown、实际输入、实例 0 下载及失败结论 | `ui/runContent/value.dom.test.jsx` 4 项 |
| 直接影响的已有链路 | `layered/layered.dom.test.jsx` 11 项、`capabilities.dom.test.jsx` 8 项、`navigation.dom.test.jsx` 1 项、`detail/embeds.dom.test.jsx` 13 项、`detail/liveTranscript.dom.test.jsx` 4 项、`detail/transcript.dom.test.jsx` 3 项 |
| 下载路径前缀、匿名入口、同源作用域和实例索引 | `fleet/download.test.js` 8 项 |
| DAG 刷新后节点仍可见，长卡片与连线保持一致 | `dag/graph.dom.test.jsx` 的 `keeps measured nodes visible when a snapshot refreshes without another resize`；`dag-process.js` 四种屏宽覆盖首次打开、连续刷新及隐藏后返回 |
| 大字段 UTF-8 分段、原执行归属及消息刷新模式 | `fleet/fleet.dom.test.jsx` 定向选择 3 项，其余用例未运行 |

以上定向测试通过，共 83 项 SPA 用例、8 项 Rust 用例；未执行全量测试。Rust 执行 `cargo clippy -p opencoder-control -p opencoder-store -p opencoder-brain --all-targets -- -D warnings` 通过，相关 Rust 文件格式检查通过。

显示验收使用 [history.js](../../../scripts/acceptance/brain/ui/history.js) 的通用合成记录，检查 1920、1280、768、390 像素宽度，28 张截图与几何检查通过；覆盖翻页返回、主详情、子执行、调度依据、实际输入和子执行返回后的滚动恢复，无未知请求或浏览器异常。该验收仅验证显示及导航。结果保存在仓库外 `/tmp/brain-history-final-ui/`。

真实进程验收入口为 [history/main.py](../../../scripts/acceptance/brain/history/main.py)：使用独立 Server、Host、Runtime 和 runc，只有模型回答使用确定性数据。运行包含两层、同层 Agent/DAG 并行、一次返工、两轮共六个子执行；核验层屏障、冻结名称、实际输入、历史评估失效与真实产物摘要。浏览器在四种屏宽检查详情页签、任务切换、返回、全屏画布及带服务路径前缀的下载；结果仅在所有步骤通过后写入 `result.json`。该验收已通过：两轮六个子执行、44 项浏览器检查、真实文件下载校验以及四种屏宽的画布交互。合并能力选择修复后，下载、导航、能力选择和计划编辑共 22 项定向用例再次通过。这份独立环境记录不替代正式发布与观察证据。

线上只读验收使用 [history-live.js](../../../scripts/acceptance/brain/ui/history-live.js)，按完整执行 ID 和 operation ID 定位表格行，避免相同前缀误匹配。检查实际实例版本、静态资源摘要、四种屏宽、详情导航和真实产物下载；窄屏同时检查基本信息的可读宽度。历史输入读取若明确返回可重试超时，使用页面重试按钮读取同一执行 ID，保存失败截图和恢复记录；连续失败仍使验收失败。

DAG 尺寸修复补充运行 `dag/graph.dom.test.jsx`、`dagProjection.test.js`、`fleet/detail/presentation/brain.dom.test.jsx`，共 25 项通过。新增尺寸保留用例在旧实现上失败、修复后通过；[dag-process.js](../../../scripts/acceptance/brain/ui/dag-process.js) 在四种屏宽检查首次打开、连续刷新、隐藏后返回、卡片高度及连线位置，共 12 项通过。运行历史显示与导航的 28 项检查再次通过。

包含以上修复的发布阶段已在两套正式环境完成验收：本机页面 56 项检查、远端页面 32 项检查，各有 3 个实际下载校验通过，均未触发重试。两端分别完成发布后的 900 秒观察；待所有验收执行结束后，再固定有效配置、版本、进程和历史执行，连续观察至少 900 秒通过。过程图刷新、窄屏阅读、原执行归属和真实下载均有对应证据。后续独立目录维护继续更新整合版本，最终统一发布状态以仓库外回执为准，不能将这一阶段的窗口视为后续版本已经验收。

SPA 构建完成并更新随仓产物；`scripts/check-spa-drift.sh` 验证构建一致性。文档和链接检查范围仅包含本次修改内容。

## 相关说明

- [大脑调度与运行历史](../../brain/index.md)
- [运行协议](../../../docs/brain-orchestration.md)
- [Web 模块](../../../agents/web/index.md)
- [Control 模块](../../../agents/control/index.md)
- [Store 模块](../../../agents/store/index.md)
