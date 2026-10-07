# 大脑调度真实验收

主场景通过当前公共 Server、真实模型和原生 DAG 容器验证：创建运行唤醒大脑；两个并行测试只完成一个时，至少 30 秒不继续调度；人工输入先落事件，再触发引导；最后一层进程正常退出但 `passed:false` 时，大脑根据失败数据修正源码，再验证同一新版本。

测试源码的初始表达式是 `abs(a) + abs(b)`。普通正数和零值测试通过，负数边界测试真实失败。模型只看到业务目标、节点能力和执行证据。`1→2→3→1→2→3`、两轮和八个子执行只写在验收断言中，不写进模型指令。两次源码的 SHA-256、归档文件、每个测试的实际结果、独立工作区和容器身份均须一致。

## 运行

需要 Python 3.10+、静态 C 编译器、`runc`、systemd、部署配置中的凭证文件，以及可用的当前模型和资源服务。凭证只从配置指定的文件读取，不传入命令行、不写入证据或 Git。每次运行必须使用新的证据目录。

```sh
python3 scripts/acceptance/brain/scheduling/main.py \
  --config /etc/opencoder/server/opencoder.json \
  --expected-commit <线上完整提交 SHA> \
  --out /var/tmp/brain-scheduling-<本次唯一编号> \
  --case closed-loop
```

`--case all --wait-ready 1800` 执行完整套件；`--case controls contracts` 可以选择多项。版本校验依据发布状态和正在运行的 Server、Runtime 可执行文件，不根据工作目录 HEAD 推断。程序不发布代码，也不把工作区尚未发布的规则当作线上能力。

完整套件包含停止 Server、杀死并恢复 Worker、注入模型回答、修改本节点并发上限等操作，应在用户已授权当前机器故障演练时执行，并由独立 systemd 作业托管。故障阶段最多等待 30 分钟，要求其他任务自然结束；精确核对运行归属，发现其他任务便停止注入，不会取消它们。进程故障期间冻结准入，结束后恢复；排队测试恢复原调度设置。模型异常只改变本次测试根运行的冻结传输地址，不修改生产模型配置、鉴权数据、重试次数或截止时间。

强杀 Worker 前还会只读检查本地 libSQL 的 `capacity_queue`。当前版本若有尚未结算的运行名额，重启会被 Host 拒绝，测试便记录 `NOT_RUN`。因此 `process-recovery` 在暂停的子任务结束后重启；`correction-deadline` 中模型调用进行期间的强杀，需要产品先具备已验证的名额恢复路径。测试不会删除数据库名额来制造恢复成功。

## 覆盖与证据

| 用例 | 主要断言 |
|---|---|
| `admission`、`definitions` | schema 7、字段与规模边界、不可变版本、固定嵌套版本、深度上限、显式不兼容节点拒绝 |
| `closed-loop` | 三类事件、等待期间不轮询、整层屏障、完整失败结果、跨层整改、修改后的同版本复验、历史记录 |
| `controls` | 暂停只停调度、暂停中输入不唤醒、恢复处理输入、取消及子执行回执 |
| `round-budget`、`same-layer-repair` | 预算耗尽不创建新任务、增加预算后恢复、同层重做消耗新轮次 |
| `six-types` | Agent、Operator、Team、DAG、TODO、固定版本嵌套 Brain；父 operation 与所属节点 |
| `receipt-replay` | 重复/迟到终态、迟到准入拒绝、旧 generation、已结束运行只读 |
| `failure-barrier` | 单任务失败后等待同层任务，不提前判断、不自动重试或取消同层任务 |
| `oversize-output`、`contracts` | 16 KiB 限制、原始失败数据保留、裁剪标记、必填输入/输出、false/0/[]、登记产物、缺失 JSON pointer |
| `frozen-contract` | 能力库变更不影响已受理运行、相同请求重放、不同行为占用同一 ID 被拒绝 |
| `missing-prerequisite`、`live-steering` | block 后人工补充条件、向运行中的 Agent/Operator/Team 引导、DAG 不接收即时引导 |
| `context-capacity` | 当前节点容量特性、超限准入 413、不创建运行、累计人工输入超限不改变状态 |
| `dispatch-retry` | 5xx/408/423/429 保持同一创建意图、422 终态、重复拒绝去重、Server 离线提交与重放 |
| `process-recovery` | 当前 Server 和 Worker 实际重启、冻结决策与意图不变、执行 ID 不变、继续完成 |
| `correction-budget`、`correction-deadline` | 非法决策最多三次、不派发；Worker 重启不重置五分钟截止时间或已消耗次数，分别记录结果 |
| `capacity-queue` | 满载显式选点受理后排队、容量释放后执行、排队不换节点、不重复执行 |

CPU 比例排序、预留量竞争、Team 在下一次成员发问时应用引导等精确内部时序，还需同时通过既有 Rust 集成回归；真实模型用例不能替代这些确定性断言。入口见 `crates/core/src/fleet/scheduling.rs`、`crates/brain/tests/milestone.rs`、`crates/worker/tests/brain_scheduler_v4.rs`、`brain_guidance_e2e.rs`、`brain_contracts.rs`、`brain_dispatch_failures.rs`、`brain_nested.rs`、`brain_server_restart.rs`。

`result.json` 逐项记录 `PASS / FAIL / NOT_RUN`。前提不足也会列出每个未运行项并以非零状态退出；不会把未执行算作通过。`scope` 和 `full_suite` 区分单项验收与全套验收。HTTP 非 JSON 错误保留状态和正文。根运行准入遇到超时或暂时拒绝时，最多用完全相同的 ID 和请求尝试三次，每次 HTTP 结果都保留；明确输入错误立即失败。失败批次不能被重跑覆盖。

证据目录还包含 `baseline.json`、`final-runtime.json`、`http/`、`requests/`、`runs/`、`executions/`、`closed-loop/`、`faults/` 和 `cleanup.json`；`suite-source/` 与 `suite.json` 保存本批验收代码及文件摘要，避免后续修改改变历史依据。清理通过本次固定节点的 Runtime 核对归属，只取消本次创建且未结束的运行，并等待子执行的终态回执，不删除历史证据。若环境不可达而无法清理，最终报告明确保留 `cleanup_error`。

```sh
python3 scripts/acceptance/brain/scheduling/report.py /var/tmp/第一次 /var/tmp/第二次
python3 -m unittest discover -s scripts/acceptance/brain/scheduling/tests -v
```

汇总同时保留最近结果和历史最后一次成功。旧成功不会抹掉新失败或未执行项；不同线上提交的结果不能拼成一次通过。

验收脚本自身的单元测试已接入 `.github/workflows/brain-e2e.yml`，只使用本地临时数据。真实服务与模型用例通过上述命令单独运行。

`decisions/` 保存每个 generation 最后一次模型提议及当时的层位置、校验错误，用于定位无效派发；不复制激活配置或冻结能力中的凭证。同一 generation 中较早的提议已被运行时覆盖，不在这份证据中。

## 页面验证

使用仓库 SPA 的 Playwright 依赖，对已完成的真实 `closed-loop` 批次运行：

```sh
node scripts/acceptance/brain/scheduling/browser/verify.js \
  http://127.0.0.1:18081 /etc/opencoder/server.token /var/tmp/本次证据目录
```

页面验证不拦截 API：打开两轮的八个真实执行，检查 DAG 日志、对应版本和业务结论，并保存四种宽度的截图。它有独立的 `browser/result.json`；服务不可达或未运行时，不能声称页面验收通过。

页面重试时传入第四个参数指定新的输出目录，保留之前的结果和截图。
