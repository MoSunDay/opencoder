# 项目工作台与执行留存验收

项目汇总所属专项，专项组织 TODO；TODO 的看板位置决定项目和专项的完成进度。
执行完成不会自动把 TODO 移到“已完成”。TODO 可以多次指派，每次保留独立执行引用；
结论从所属节点实时读取，步骤、消息和调度过程由同一原生执行详情提供。
Agent 页的“全部执行”和 TODO 打开的是同一个执行。

项目、专项、TODO 与 Agent 页泛化能力的完整对接由以下入口验证：

~~~sh
PLATFORM_BIN_DIR=/path/to/matching/bin \
node scripts/acceptance/project/capabilities/main.js /path/to/matching/rootfs
~~~

该场景已纳入全站 UI 验收。浏览器创建一个项目、一个专项和四个 TODO，
分别派发给 Agent、Operator、DAG 和 schema 7 大脑计划；大脑先并行执行
Agent、Operator，再将实际输出绑定给 DAG 验证。使用真实 Server、节点、
HTTP、持久化与 runc，仅模型响应是确定性夹具。
隔离模型显式配置 256K 上下文容量，满足三个节点的完整证据预留；
正式环境使用自身模型配置，容量不足会保留启动错误并判定验收失败。

验收核对输入标记与计算结果、执行 ID、TODO 与全部执行页面中的同一执行、
执行状态、刷新后实时结果、多次指派、幂等关联和手动看板进度。
隔离环境还覆盖四类执行实际受理后回执丢失、原界面使用相同 ID 和输入重试，
以及模型失败、取消、空结果和节点离线错误；节点恢复后可重新读取结果。
任务记录不复制结论、步骤、消息或调度过程。
计算节点必须返回正确的 `marker`、`sum=10` 和 `passed=true`，
大脑的下游输入还须与实际上游完整输出一致；原生结果可以是 JSON 对象或包含 JSON 的文本。

正式环境复用同一套浏览器步骤，使用真实模型。它仅创建具名验收数据，
保留项目和执行供人工复查，不执行隔离环境中的故障注入：

~~~sh
node scripts/acceptance/project/capabilities/main.js \
  --live http://server:18081 --token-file /path/to/token \
  --node-id node-id --output /path/to/new/evidence
~~~

失败后可用同一组参数追加 `--resume`，从已保存执行 ID 继续验证。
`state.json` 保存实际版本、对象 ID 和已完成步骤；只有全部断言通过才生成
`report.json`。失败保存截图、HTML 和错误；凭证不写入证据。
当前源码与正式环境的版本分别记录，隔离通过不表示已发布。

当前构建也可以在独立 Server、节点和 NFS 中使用已有真实模型配置：

~~~sh
PLATFORM_BIN_DIR=/path/to/matching/bin \
node scripts/acceptance/project/capabilities/main.js \
  --rootfs /path/to/matching/rootfs --model-config /path/to/private/model-config.json \
  --observe-model
~~~

配置使用现有 provider、model 和上下文容量；不启动模型夹具，也不注入模型故障。
配置文件与临时工作目录应保持私有，配置里的凭据不能提交到 Git。
`state.json` 区分 `real` 与 `fixture`，记录配置模型；运行资料中的模型、
响应 token 用量和实际输出需一同复核。服务进程与 DAG 容器均遵守现有
`NO_PROXY` / `no_proxy`，不增加新的网络配置字段。

`--observe-model` 仅在隔离的真实模型场景中可用。它逐字节转发请求和响应，
不替换模型输出；回执记录实际响应模型、token 用量与请求/响应摘要，
不记录授权头或完整消息。模型网关可能返回与请求模型不同的模型，验收按实际响应记录。
网络直连与代理例外须另跑一次不带此参数的场景，不能由本地转发验证代替。
原生结果中的嵌套证据、Markdown JSON 与转义字符串都按完整 JSON 解析，
实际结果错误时不能因旁边有正确的预期值而通过。

工作台界面可单独使用可变 API 夹具验收，无需启动 Server 或 Node：

```sh
node scripts/acceptance/project_workbench_ui.js
```

该脚本使用当前 SPA 构建产物，检查项目、专项、TODO、Tag 四个表格及 Tag 编辑表单在 1920、1280、768 和 390 像素宽度下的展示，项目详情与专项看板抽屉，筛选后拖动的完整顺序，Tag 分组中的同一 TODO 同步，保存失败回退，以及项目与 Tag 增删改。Tag 验收覆盖独立专项归属、改名并移动至项目、重名失败保留填写内容。默认在 `/tmp/opencoder-project-ui` 保存 `receipt.json` 和截图，可用 `PROJECT_UI_ARTIFACTS` 指定输出目录。浏览器需要 SPA 的 `playwright-core` 依赖及其配套的 Chromium，也可通过 `CHROME_PATH` 指定路径。此验收使用临时 HTTP 服务与夹具凭证，不访问生产数据；存储迁移与真实接口由 Rust 测试验证。

在独立临时目录中启动构建后的 Server、两个 Node、只读 NFSv3 Agent 资源池、HTTP 模型夹具和 Chromium，覆盖项目、关联项目与独立专项、分组 TODO 与 backlog 的真实浏览器创建，以及通过原生能力界面发起执行、关联指派记录和查看结论。模型响应由本地夹具确定性提供，不调用外部 LLM。

先完成 SPA 构建与 `cargo build --workspace`。运行环境需要 Linux NFS 客户端、挂载权限、Python 3、支持 `Array.prototype.toReversed` 的 Node.js、SPA 的 `playwright-core` 依赖及其配套的 Chromium；旧系统 Chromium 可能不支持 `marked` 所需的 `Array.prototype.at`。临时目录所在卷须满足 Node 的存储就绪检查（至少 20% 可用空间）。

```sh
TMPDIR=/path/to/isolated/tmp \
PLATFORM_BIN_DIR=/path/to/cargo-target/debug \
node scripts/acceptance/project/main.js
```

仅验项目页与 TODO 指派闭环可加 `--workbench-only`；该模式创建独立夹具数据，核对 Agent 执行 ID、实时结果读取与浏览器记录，不运行旧项目执行回放。

若 Chromium 不在 Playwright 默认安装路径，可设置 `CHROME_PATH`。脚本为每次验收复制独立二进制并生成临时凭证，不读取生产配置或凭证。运行结束停止自身服务并卸载自身 NFS 挂载，保留临时目录中的数据与证据。

验收包含旧项目执行 API 的多次运行留存检查，其中同一 TODO 超过 25 次：稳定运行 ID 的重试、超过 64 KiB 的输入与输出、历史分页、Agent 和资源版本切换、交付文件不可变副本、模型失败、所属节点宕机后的明确恢复，以及产生部分输出后的取消。通过执行索引读取所有运行的输入、消息、事件、模型请求与响应，并与归档核对。浏览器验收新工作台的层级创建、执行关联和原生详情入口，不再寻找已移除的项目回放页。

完成上述场景后，回读全部成功运行的索引、摘要和过程清单，检查归属、终态、无重复、旧记录不变和浏览器错误。终态索引须在运行完成后 10 秒内收敛。E2E 和当前服务健康通过即完成验收，不附加固定时长观察。

标准输出的 `ready.evidence` 指向证据目录：

- `payload-audit.json`：全部运行的留存数据核对数量。
- `storage-audit.json`：以只读模式打开夹具数据库，逐字节核对全部输入、方案、输出、过程清单与消息分块后的摘要。
- `workbench-browser.json`、`project-workbench.png`：TODO 关联执行与详情入口的浏览器证据。
- `report.json`：只有全部 E2E 断言与当前健康检查通过后才写入的最终结果。
- `browser-failure.html/png` 与进程日志：失败定位信息。
