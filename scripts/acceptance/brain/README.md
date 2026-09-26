# 大脑 schema 7 E2E

主回归入口是 `crates/worker/tests/brain_browser.rs`，由 CI 的 `brain-e2e.yml` 在相关代码变更时执行，同时运行里程碑状态机与 Worker 调度定向测试。浏览器测试启动隔离的 Control、Node、持久化调度器和真实 SPA；Chromium 通过实际 HTTP 操作页面。只有大脑决策模型使用确定性响应，不依赖外部模型、生产令牌或生产数据。

本地运行：

```sh
cd crates/web/spa && npm ci && npx playwright-core install chromium && npm run build && cd ../../..
cargo test -p opencoder-worker --test brain_browser schema_seven_canvas_parallel_return_and_execution_detail -- --ignored --nocapture
```

该场景从工作台进入计划库，在画布配置两层里程碑：Coding 层并行绑定 Agent 和 Operator，测试层绑定 Operator；画出测试回 Coding 的连线并填写条件。关闭再打开草稿，提交 schema 7 计划，选择节点发布运行。确定性模型在首轮测试后回退 Coding，第二轮重新执行两层并完成。

断言覆盖：计划版本及能力绑定、可见的“类型 · 能力名称”、工作台入口、草稿持久化、四次层激活、每轮同层并行派发、全部节点终态后才越过层屏障、回退反思、六个互异执行 ID、按激活读取历史，以及同一节点两轮分别按 ID 拉取执行面板。失败时 Chromium 截图和页面 HTML 写入 `/tmp/opencoder-brain-v7-browser-*`；CI 会保留这些证据。

`layered-production.js` 是独立的可选真实模型验收，覆盖 Agent、DAG、Team、Operator、TODO 和嵌套 Brain 六种执行类型；它会创建具名测试计划和能力，不属于每次提交的 CI 门槛。发布时使用已审核环境显式执行，避免在日常回归中写入生产数据。
