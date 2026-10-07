# 大脑 schema 7 E2E

主回归入口是 `crates/worker/tests/brain_browser.rs`，由 CI 的 `brain-e2e.yml` 在相关代码变更时执行，同时运行里程碑状态机与 Worker 调度定向测试。浏览器测试启动隔离的 Control、Node、持久化调度器和真实 SPA；Chromium 通过实际 HTTP 操作页面。只有大脑决策模型使用确定性响应，不依赖外部模型、生产令牌或生产数据。

本地运行：

```sh
cd crates/web/spa && npm ci && npx playwright-core install chromium && npm run build && cd ../../..
python3 scripts/ci/brain.py --output /tmp/opencoder-brain-acceptance prepare
CHROME_PATH="$(node -e 'console.log(require("./crates/web/spa/node_modules/playwright-core").chromium.executablePath())')" \
  python3 scripts/ci/brain.py --output /tmp/opencoder-brain-acceptance browser
```

Linux 还需 `runc`、`nfs-common`、C 编译器、`unshare` 及可免交互使用的 `sudo`（或以 root 运行）。编译由当前用户执行；镜像内校验单独提权，重启和浏览器测试在私有挂载及 PID 命名空间中执行，退出时终止残留子进程。原生阶段使用 `/tmp` 下由执行用户持有的私有临时目录，避免 Chromium 丢弃特权后无法穿过 runner 私有父目录；结束后归还调用用户并归档。镜像中的 runner 必须与测试程序版本一致。复用上述输出目录，分别执行 `milestone`、`scheduler`、`restart` 可运行其余三组测试。

`project` 阶段执行项目运行模块的全部单元与集成测试，其中 DAG 恢复用例也需要配套镜像和挂载权限。纯 libsql 存储测试由独立的 `project-store-tests` 工作流执行。

每一步保存独立的构建与测试日志；截图和 HTML 位于输出目录的 `browser-tmp/oc-brain-browser-*/opencoder-brain-v7-browser-*`。CI 无论成功失败均上传这些证据，保留退出码和失败摘要。浏览器阶段记录 Chromium 进程日志；即使页面崩溃、截图或 HTML 读取失败，`failure.json` 和控制台仍保留原始异常。

该场景从工作台进入计划库，在画布配置两层里程碑：Coding 层并行绑定 Agent 和 Operator，测试层绑定 Operator。关闭再打开草稿，提交 schema 7 计划，核验保存时自动补齐前进和回退路径，再选择节点发布运行。确定性模型在首轮测试后回退 Coding，第二轮重新执行两层并完成。

断言覆盖：计划版本及能力绑定、可见的“类型 · 能力名称”、工作台入口、草稿持久化、四次层激活、每轮同层并行派发、全部节点终态后才越过层屏障、回退反思、六个互异执行 ID、按激活读取历史，以及详情抽屉的轮次表格和同一节点两轮分别按 ID 拉取执行面板。

`layered-production.js` 是独立的可选真实模型验收，覆盖 Agent、DAG、Team、Operator、TODO 和嵌套 Brain 六种执行类型；它会创建具名测试计划和能力，不属于每次提交的 CI 门槛。发布时使用已审核环境显式执行，避免在日常回归中写入生产数据。
