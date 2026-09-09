# 真实业务 E2E（固定节点）

默认使用输入完整的受控评测及固定代码提交，配合真实 Codex，验证业务 API → 单并发 FIFO Node → Runner → 业务发布 → Web 三层折叠、刷新回放、文件下载。

受控回归先通过真实 Git 命令核对请求分支头、目标可达性及基线祖先关系，再将证据同时纳入 workspace 提交和上下文文档。这样即使 Runner 使用不带分支引用的固定快照，复核模型仍能确认提交归属。

```sh
python3 scripts/acceptance/business/main.py \
  --root /root/.cache/opencoder-e2e/20260909-acceptance \
  --release /absolute/path/to/immutable/jy-hub-agent-release \
  --platform-bundle /absolute/path/to/opencoder-platform-bundle \
  --scenario positive
```

要求本机安装 systemd、NFS 客户端、opencoder-agent、真实 Codex 及 Playwright Chromium；平台四个二进制必须来自同一已验证 bundle。业务配置采用 `common.py` 指定的本机路径；`--scenario historical` 另外使用真实 case 5 和固定的 `jianying-openagent-api` 提交。Server 必须支持超过 60 字节的 NFS 资源路径。启动前会逐文件验证完整 skill 包经过 NFS 后的哈希，不能用补写 Node 快照代替真实导出。

Manager/飞书投递及新版 Viking 工单创建使用本地接收器，兼容旧群投递和新工单接口；评测及回归分析始终经 opencoder，未知投递接口直接拒绝启动。无需建单只有在同一任务分析完成且业务返回 `not_required` 时才可验收；需要投递时必须有匹配该任务的本地回执。模型、输入下载、Git 版本解析、实际回归命令、结果校验和 Web 均真实运行。评测原始输入缺失的 trace workspace、历史执行版本等仍作为证据缺口报告，不补造数据。业务执行完成不代表被测提交必然通过准出。

历史场景可用 `--evaluation-request /absolute/path/request.json` 显式提供已经核实的 trace 路由和版本信息，原始失败证据保持不变。回归分支证明来自复制仓库的实际 Git 分支头和祖先关系，不作为历史评测部署版本的证明。Runner 使用私有账号副本，包括当前 bytedcli 的 XDG 数据目录及旧版 CLI 目录；显式提供的 `BYTEDCLI_USER_CLOUD_JWT`、`FORNAX_BYTED_JWT_TOKEN` 可沿用，通用 `JWT_TOKEN` 不作为 bytedcli 身份。不切换或修改宿主登录状态，凭证不进入公开回执。

原 `/root/workspace` 只读使用，独立 Git 对象、上下文、账号副本、缓存、数据库均在本次 `runtime/`。Runner 使用挂载命名空间保护原目录；评测新建的 systemd 模型单元通过仅本次 Node 可用的包装器再次显式保护原目录。回归使用既有 OverlayFS/chroot 工作区。`ProtectSystem=strict` 本身不能保证 `/root` 只读。

回归沙箱同时只读映射宿主 Go 的实际安装目录，支持 `/usr/local/bin/go` 指向 `/data00` 的安装方式；Go 模块访问参数复制到私有配置，编译和模块缓存仍写入本次沙箱。

启动时独立复制宿主 Go 已下载的模块归档，在私有目录使用固定提交的 `go.mod/go.sum` 补齐依赖后保存哈希清单。缺失的内部模块通过已有 SSH 身份读取，URL 转换只写入临时 HOME 的 Git 配置，严格校验已有 host key。每项测试在启动前获得自己的缓存副本，仍实际准备依赖、编译和执行；不复制编译结果，不修改宿主认证配置。无法获取的依赖直接阻止历史场景验收启动。受控正例仅使用 Node.js 实际断言，不依赖业务 Go 环境。

依赖准备继承已有 HTTP/HTTPS 代理设置，值只写入本次私有配置；实际测试阶段仍切断外部网络。

`--scenario historical --regression-fixture metricw-offline` 显式启用固定目标的离线 metrics 夹具。它校验目标依赖为 `metricw v0.0.4`，使用目标声明的精确 Go 工具链，并在副本中加入缺失的本地日志配置。测试阶段只允许 loopback，HTTP 夹具仅响应 metrics sampler/clip 的缺省配置请求，其他请求会使验收失败。Go 的兼容链接参数 `-ldflags=-checklinkname=0`、工具链/包装器哈希、配置和实际请求均写入 `regression-fixture.json` 及测试回执。业务代码、测试断言和依赖准备命令保持原样；使用该夹具的结果只证明所声明测试环境下的行为，不替换无夹具的历史结论。

`evidence/` 保留原目录前后清单、各仓 HEAD/status、排队状态、模型实际挂载、消息、带哈希的报告和下载截图。原目录有并发写入时如实列出差异，不回滚其他任务的内容。正常结束停止本次服务、卸载 NFS，保留本次 `runtime/` 内的数据库、缓存和副本。`--retain-on-failure` 仅供诊断，使用后必须显式完成清理。

完成诊断并保存证据后，可执行 `python3 scripts/acceptance/business/main.py --root <本次目录> --cleanup`。清理核对创建时记录的目录身份、服务归属、进程及打开的文件和各挂载命名空间，保留本次 runtime；缺少归属证明、目录被替换或仍被占用时报告失败。保留所有数据库和运行目录，完成的运行目录不能重用。

用户已授权销毁本轮测试副本时，启动或清理命令显式加 `--destroy-runtime`。仅在服务、测试输入服务进程和挂载全部退出、原目录审计完成后删除归属校验通过的 `runtime/`，保留 `evidence/`；不会触及运行目录以外的数据库。默认仍保留运行数据。

固定节点和 workspace 调度由上述真实业务流程验证。runc 模式通过 `scripts/acceptance/runc_scheduling/main.py` 单独验收：两个节点分别配置相同 rootfs 和 Wasm 模块，轮流占满一个节点，验证无节点绑定的任务在另一个节点执行，固定节点任务则先 pending，取消占用任务后在指定节点执行。它验证新任务的节点选择，不代表运行中任务可以迁移，也不代表未配置运行时的节点可接单。

先用 `scripts/prepare-dag-rootfs.sh /absolute/private/path/rootfs` 准备 rootfs，再执行：

```sh
python3 scripts/acceptance/runc_scheduling/main.py \
  --root /root/.cache/opencoder-e2e/20260909-runc-acceptance \
  --platform-bundle /absolute/path/to/opencoder-platform-bundle \
  --rootfs /absolute/private/path/rootfs
```

该验收同样只使用私有状态并保护原 workspace；已授权回收本轮临时数据库和副本时加 `--destroy-runtime`。保留 OCI 配置、实际容器状态、节点选择、排队和产物证据。

仓库进程测试使用 target 目录内的配套二进制，执行全量回归前先运行 `cargo build --workspace --bins`。通过脚本启动验证时显式关闭标准输入（例如 `cargo test --workspace </dev/null`），避免搜索工具把调用方脚本当成输入流。

`result.json` 分别记录 `platformPassed`、`businessQuality` 和整体 `passed`。只有真实执行、Web 及有效业务结论全部满足才整体通过。依赖失败、未执行断言和缺失证据不会变成成功；历史失败证据保持原样。验收完成后不附加固定观察等待。

验收工具自身的回归入口：`python3 -m unittest discover -s scripts/acceptance/business/tests` 和 `node --test scripts/acceptance/business/tests/delivery.test.mjs`，覆盖真实 Git 祖先校验、输入拒绝、私有身份、投递隔离、质量门禁及数据保留。
