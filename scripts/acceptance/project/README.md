# 项目工作台与执行留存验收

在独立临时目录中启动构建后的 Server、两个 Node、只读 NFSv3 Agent 资源池、HTTP 模型夹具和 Chromium，覆盖项目、里程碑、关联项目与独立专项、分组 TODO 与 backlog 的真实浏览器创建，以及 Agent 原生提交、自动关联与详情入口。模型响应由本地夹具确定性提供，不调用外部 LLM。

先完成 SPA 构建与 `cargo build --workspace`。运行环境需要 Linux NFS 客户端、挂载权限、Python 3、支持 `Array.prototype.toReversed` 的 Node.js、SPA 的 `playwright-core` 依赖及其配套的 Chromium；旧系统 Chromium 可能不支持 `marked` 所需的 `Array.prototype.at`。临时目录所在卷须满足 Node 的存储就绪检查（至少 20% 可用空间）。

```sh
TMPDIR=/path/to/isolated/tmp \
PLATFORM_BIN_DIR=/path/to/cargo-target/debug \
node scripts/acceptance/project/main.js
```

若 Chromium 不在 Playwright 默认安装路径，可设置 `CHROME_PATH`。脚本为每次验收复制独立二进制并生成临时凭证，不读取生产配置或凭证。运行结束停止自身服务并卸载自身 NFS 挂载，保留临时目录中的数据与证据。

验收包含旧项目执行 API 的多次运行留存检查，其中同一 TODO 超过 25 次：稳定运行 ID 的重试、超过 64 KiB 的输入与输出、历史分页、Agent 和资源版本切换、交付文件不可变副本、模型失败、所属节点宕机后的明确恢复，以及产生部分输出后的取消。通过执行索引读取所有运行的输入、消息、事件、模型请求与响应，并与归档核对。浏览器验收新工作台的层级创建、执行关联和原生详情入口，不再寻找已移除的项目回放页。

完成上述场景后，回读全部成功运行的索引、摘要和过程清单，检查归属、终态、无重复、旧记录不变和浏览器错误。终态索引须在运行完成后 10 秒内收敛。E2E 和当前服务健康通过即完成验收，不附加固定时长观察。

标准输出的 `ready.evidence` 指向证据目录：

- `payload-audit.json`：全部运行的留存数据核对数量。
- `storage-audit.json`：以只读模式打开夹具数据库，逐字节核对全部输入、方案、输出、过程清单与消息分块后的摘要。
- `workbench-browser.json`、`project-workbench.png`：TODO 关联执行与详情入口的浏览器证据。
- `report.json`：只有全部 E2E 断言与当前健康检查通过后才写入的最终结果。
- `browser-failure.html/png` 与进程日志：失败定位信息。
