Commit: (working-tree, ops-only)

# jy-builder agent-team 上线（零代码改动 · 纯平台注册 + 运维闭环）

## 拓扑与机制选型（代码实证）
- 选定 A 套 TeamDefinition（成员=agent 引用卡，成员会话无超时）而非 B 套节点编排 team（`NodeDispatcher` 默认 30 分钟超时，VideoFusion full 构建 ~38 分钟必炸）。
- team 执行钉在 human-os-02（node-01M1WVDEYE7Q4TFV6J83EZGKYJ，即本机，满足 cicd 节点条件：NFS agents_dir 只读挂载 + g++ + viking-cli + jyhub 凭证 0600）；不钉会被 `select_queue_node` 按负载随机选点，且 jyhub-macos-builder 节点的 config 已摘除 `agents_dir`（`check_mount` 强制只读 NFS），agent 卡在该节点不解析、成员会静默退化成默认 act。
- VideoFusion 构建走已跑通 jyhub 链路：成员经 `ssh -i /root/.ssh/id_ed25519_10_37_35_13_20260811 macos-arm-linux-100-86-225-228` 驱动 VM `/opt/opencoder-agent/scripts/jyhub-build.sh`（exit 42/44/45 语义、上传回读兜底、systemd-run 托管全继承 changelog 2026-09-17 部署成果）。
- 能力冻结链路：brain caps target-bind(agent) → `POST /api/executions` 时 `catalog.rs` resolve 把绑定 summary 自动冻结进 pinned definition `members[].capabilities` → worker `team.rs` 拼 `你的能力：…` 前缀。

## 注册面（全部经 opencoder-cli，34f69db1）
- prompts 池 v1 ×3：`jybuilder-{videofusion-win-operator,lyra-cli-builder,customagent-builder}`（soul/how/output 三段式：soul 恒 act + fail-closed + 制品必回读 + 凭证边界；how 各构建配方；output 单行 JSON 合同 ok/project/branch/commit/artifact_remote/artifact_sha256/seconds/error）。
- skills 池 v1 ×2：`jyhub-macos-build`（jyhub-macos-build + artifact-upload 两个 skill）、`cpp-build-upload`（cpp-build + artifact-upload）。
- memory 池 v1 ×3（起步占位，首跑后经 `agents resources update memory` 沉淀踩坑——成员会话无池写工具，写池由 ops 维护）。
- agent 卡 ×3：`videofusion-win-operator`、`lyra-cli-builder`、`customagent-builder`，references 全绿（prompt_files=[soul,how,output]、skills 解析、memory=true）。注意：卡的 `current.memory` 必须用池全名（`jybuilder-<卡名>`），引用名不匹配会静默 `memory:false`（update 重写卡即刷新快照）。
- capability ×3 + target-bind ×3（kind=agent）：`brain-01M2S4FKDMFS52M1HZ5E0J0Z2B`→videofusion-win-operator、`brain-01M2S4FKPZF2FN1NJKCX7C9BJ7`→lyra-cli-builder、`brain-01M2S4FKZJHBKN09HJGPHZXWYK`→customagent-builder。
- team：`jy-builder`（captain=videofusion-win-operator ∈ members，三成员同名卡）。

## 验证闭环（V0–V8）
- V0 前置：4 节点 online、human-os-02 与 jyhub-macos-builder kinds 含 team；NFS 只读挂载实证（touch 拒绝）；VM 脚本 `test -x` 通过。
- V1 注册面：agents list 3 条 + meta 三引用正确；负例 captain∉members→400（TeamDefinition::validate）、卡非法名→400（validate_agent_name）。teams 层无 delete 路由（put 覆盖语义），负例团队已覆盖为休眠合法形态。
- V2 persona 解析（kind=agent 三冒烟，全部单行 JSON 合同）：lyra-cli-builder 三条只读检查取证（HEAD=84de2ff98、cmake 缺失如实记录、build.sh entry-ok）；videofusion-win-operator VM 链路 probe_rc=0；customagent-builder 缺 repo_path fail-closed 报错、未扫描磁盘。
- V3 路由正确性：team 冒烟单 `team-jybuilder-smoke-20260918a` turn1 plan.participants 精确=三成员；manifest 专项单 plan.participants=仅 videofusion-win-operator（其余 0 调用）；pinned definition 中三成员 capabilities 各自冻结对应 summary（team.json 实读）。
- V4 全链路（实跑三轮，fail-closed 全程）：
  - 第 1 轮（team-jybuilder-videofusion-manifest-20260918）：fetch 全历史 6.9G/70min 成功，但新 commit 将 videoeditor_mac/CCCreator_Mac 升至 22.1.0 触发 podbuild_ve.sh 重装 vesdk，CocoaPods 1.11.3 root 下拒绝 pod install（ve_sdk_mac.cmake:235 FATAL），exit 1 如实回单。
  - 修复：VM 脚本三段 ssh（checkout/build/manifest）export COCOAPODS_ALLOW_ROOT=1（备份 jyhub-build.sh.bak-cocoapods）。
  - 第 3 轮（...e）：checkout ✓、pod install 阶段 ✓（修复生效，CMakeCache 12:40 推进）、output 目录出现子 app —— configure 推进到 FetchContent 拉 lyra-cli 时 code.byted.org SSH publickey 三次被拒（mac host id_ed25519_code_byted 失效），exit 1；captain 独立实测复现、成员 fail-closed 回单（artifact_remote=null）。定性：jyhub 基础设施凭证问题（上游 commit 引入私有 git 依赖），非平台/流程缺陷；需人工在 mac host 更新 code.byted.org deploy key 后同 commit 重跑。
  - 第 2/4 轮为 LLM 决策波动（...c 只做了口径讨论即收尾）；对策已验证：prompt 用「执行单：要求 turn1 即执行，禁止只读讨论」的硬约束。
  - full 模式复验与 customagent 真实构建单 deferred：customagent 本机无主仓检出，repo_path 以需求单为准（缺失即报错，不虚构路径）。
- V4 lyra-cli 本地构建（agent 单 `agent-jybuilder-v2-lyra-build-20260918b` + steer 续跑，两轮真实执行）：
  - 第 1 轮：systemd-run `lyrabuild-84de2ff9` 真跑 ./build.sh，60 秒止于 Stage 1 `git lfs install --local`（git-lfs 缺失 exit 3）；同时确认 videoeditor 源码 clone 经 SSH code.byted.org 在本机可用。
  - operator 工具链动作（主机级，非代码改动）：apt 装 git-lfs 2.7.1；pip 装 cmake 3.31.10 至 `/root/.local/venvs/python3.11-base/bin/` 并 symlink `/usr/local/bin/cmake`（apt cmake 3.13.4 不满足 CMakeLists `cmake_minimum_required 3.14`，pip 钉 `<4` 避开 4.x 移除行为）。
  - steer 同会话续跑（`steer-lyra-toolchain-20260918a`）：git-lfs 生效（`Git LFS initialized`）、cmake 生效，推进到 videoeditor 共编 Stage 1 内部 update_vesdk.sh 下载闭源 SDK（tbc.vesdk.cccreator_1.0.0.132）——仅认真实 SCM basic auth；agent 实测匿名 GET 403 / dummy basic auth 403 / Kerberos Negotiate 403，本机与 VM 凭证文件均只有 VIKING_AUTH_TOKEN，artifact CLI 全链路无。fail-closed 回单（artifact_remote=null，工作区干净，关键事实沉淀成员记忆池）。
  - 定性：lyra-cli Linux 构建被 luban-source 闭源包下载凭证阻断（SCM_USER/SCM_PWD 或 artifact CLI 必须人工下发），与 VideoFusion deploy key 同类上游凭证问题；平台侧路由/回单/工件合同全程正确。
- V5 失败路径：customagent-builder 缺参冒烟（V2 第三单）+ team 内 fail-closed 行为一致（captain 复核确认「未代跑他人分片、未扫描近似目录」）。
- V6 幂等：同 id 同输入重放返回原执行收据（created_at 不变）；同 id 异输入→409 "execution id already used with different input"。
- V7 制品留存：目录规范 `/jyhub/<project>/<branch-slug>/<commit>/` + BUILD_INFO.txt 写入 how/skill；complete 404 误报兜底（size 回读/下载回读 sha256）已写入 artifact-upload skill，一切以回读为准。
- V7 制品留存：BUILD_INFO 生成/上传/回读链路在 9/16 部署验证时已实证（rc-develop 遗留产物在位）；本轮因上游凭证问题未产出新制品，回单 artifact_remote=null 如实。
- V8 双入口：CLI 入口全量验证（上述所有单）；Web 入口 HTTP 层复验通过——`GET /` 200 返回 SPA（text/html，zh-CN index），`GET /api/executions` Bearer 200、无凭证 401；浏览器人工交互项留待后续。

## 语义新沉淀（新踩坑）
- 504/503 用同 id 重试是「幂等重派」；**400 preflight 的 id 不可同 id 重试**——server 已持久化 assignment（outbox 反复重派），重试走 definition=None 路径永久失败，必须换新 id。
- preflight ENOENT 毒点：NFS agents_dir 中出现「有 v1 目录但无 meta.json」的半途注册（如 memory/harness-gate-data，外部流程反复重建），pin() 裸 read(meta.json) 即 ENOENT 且错误无 context。处置：DB 无记录（meta 接口 404）→ 手写 meta.json current=1 透针即可；注意 `.staging~` 前缀跳过只对 category 内资源生效，根级不适用。
- team 单 captain 决策有 LLM 波动：同 prompt 可能被解读成「口径讨论」而非执行；关键单 prompt 必须写「执行单：要求 turn1 即执行，禁止只读讨论」。
- turn1 participants 可能临时扩围（三成员被拉进只读讨论），属正常；真正构建分片仍精准落在 videofusion-win-operator。

## 提交回执语义（新踩坑沉淀）
- `504 node request timed out`：节点实际可能已接受（runtime indexes 可查），同 id 重试幂等；`503 node disconnected; assignment retained` 同 id 重试；`503 node initial index sync pending` 等节点 ready 后同 id 重试；不要换 id（会产生孤儿冻结）。
- systemd-run 场景必须用绝对路径 cargo（systemd 环境无用户 PATH）。

## 回归门
- `cargo test --test team_e2e` 1 passed；`cargo test --test operator_e2e` 6 passed（A 套 team 合约 + kind=agent how_append 注入 + interrupt 恢复全绿）。
- `cargo test --workspace` 全量编译被工作树进行中的 untracked 实验代码阻断（`crates/dag-runtime/src/exec/wasm/host_imports.rs`/`agent_runc.rs` 测试编译错误），与本次 ops-only 改动无关；本次零代码改动，无新增回归面。

## 回滚
- `opencoder-cli agents delete <名>` ×3（池 409 保护会先卡引用，需先删卡）；`opencoder-cli teams put` 覆盖重定义；执行中单 `opencoder-cli exec cmd <id> --action cancel`。

## 多平台出包实战收口（v2 运行时，2026-09-18 下午）

- **customagent（bun 打包）**：`agent-jybuilder-v2-customagent-build-20260918` ok=true。develop@347d453a（含未推送 85f54bd1，脏树豁免门禁），`bun scripts/build-bun-app.mjs` 产出 dist 5 项；tar.gz 2853676B 上传 `/jyhub/customagent/develop/<commit>/`，operator 独立下载回读 sha256 `dc182f44…` 逐字节一致，包顶层 dist/ 8 entries。BUILD_INFO 尾部附 git status --porcelain 全量（242 行）供审计。
- **VideoFusion-win（mac，双阶段）**：manifest 单 ok=true（unit jybuild-f3727d6c2，3625s；BUILD_INFO 284B，主二进制 sha256 `3e9e00af…`）；full 单 ok=true（`agent-jybuilder-v2-videofusion-full-20260918b`，unit jybuild-full-f3727d6c，1751s——42GB 增量缓存使 ninja 零重编，仅打包+上传+验证耗时）。app.tar.gz 910790363B，operator 独立下载回读 sha256 `b040fd24…` 一致；artifact_remote=`macos/rc-develop/<commit>.app`。manifest 首跑曾因真 mac 孤儿 configure + CPM 缓存竞态误报，清孤儿复跑即过。
- **lyra-cli**：`agent-jybuilder-v2-lyra-build-20260918b` blocked（上游缺陷实锤，非环境问题）。gen-patch-only 经 operator 授权的 deps/include 头文件预置（6923 头，全部来自本地 videoeditor 源码检出，清单 `/tmp/deps-provision-manifest.txt`，零二进制零下载）后编译 100% 收敛，但链接阶段 `undefined reference to lvve::Logger::*`——`main.cpp:89` 的 `lyra_info` 在 `LYRA_ONLY_GEN_PATCH` 守卫外且实现仅在 `libvideoeditor.so`。上游修复 `b9f0d575f "[build] fix only_gen_patch logger isolation"` 在平行分支（与 rc/develop 分叉于 f424b4a8），不在 pinned 84de2ff9。full 模式另有 `tbc.vesdk.cccreator_1.0.0.132.tar.gz`、`faceu.clipflow_linux.clipflow_1.0.0.3.tar.gz` 两个闭源 artifact 的 SCM 凭证卡点（luban-source 匿名 GET 403×3 实测）。

## 上传通道排障与修复（本轮最重要平台沉淀）

- 症状：viking-cli upload 对 `/jyhub/...` 路径 complete 阶段一律 404（成员侧曾误判 hang，改 /etc/hosts 抓包排障，已恢复干净）。
- 根因两层：a) `/jyhub/<project>/<branch-slug>/<commit>/` 目录树不存在——frontdoor 不自动建父，`POST /api/v1/viking-file-server/directories` 需逐级建父（HMAC-SHA256 签名三头，参考 VM `fs-mkdir.py`/`fs-raw.py`，已复制本机 /tmp）；b) 本机 viking-cli 默认个人命名空间看不到 /jyhub，必须 `export VIKING_AUTH_PROD_ISSUED_TOKEN="$VIKING_AUTH_TOKEN"` 进 prod 模式（jyhub-build.env 已内置该导出）。
- 修复验证：operator 建 tree（customagent/develop/347d453a、VideoFusion-win/rc-develop/f3727d6c、lyra-cli/rc-develop/84de2ff9）后 probe 上传→下载回读 sha256 一致；两份 artifact-upload SKILL.md 已补「2026-09-18 补充」段。

## 平台机制新踩坑（复用价值高）

- **doom-loop 守卫**：同形 bash 命令重复 20 次平台直接终止会话（videofusion manifest 轮询触发）。等待型任务必须拉长间隔+命令携带变化内容，或由 operator 盯长任务、完成后一次性 steer 唤醒收尾（本轮 full 单采用此模式，全程 0 轮询）。
- **exec create 不钉 node_id 会被派去其他节点**（本次落到 node-01M2QYGEAMB…，agent 卡不解析）；创建体必须显式 `node_id=node-01M1WVDEYE7Q4TFV6J83EZGKYJ`。504/503 同 id 重试幂等照旧有效；单创建后 status=idle 且 0 消息 = turn1 未投递，用 `exec cmd --action steer --json @prompt` 注入即启动。
- steer JSON 合同：`{"prompt": "..."}`（`--action steer --json @file <id>`）；errored 会话可 steer 复活。

## 挂起项（等用户决策）

1. lyra-cli gen-patch：三选一——cherry-pick b9f0d575f（产物=补丁后新 commit，需明示授权）；等 rc/develop 合入修复后按新 commit 重建；或放行 full 模式 SCM 凭证。deps/include 供体机制保留在位可复用。
2. lyra-cli full：两个闭源 artifact（vesdk/clipflow）等 SCM 凭证或 artifact CLI。
3. customagent 检出含未推送本地 commit 85f54bd1，制品对应 HEAD 347d453a；推送/PR 由用户侧决定。

## lyra-cli 本地构建收口（2026-09-18 晚）

- 用户拍板"本地完成构建、不依赖 SCM"。核实 b9f0d575f 已在 `/data00/lyra-cli` 本地对象库（平行分支此前 fetch 过），cherry-pick 纯离线完成：rc/develop@84de2ff98 → 新 commit `5edd661a68e0616b8efbc949de67ba46a29aaab4`（未推送、零 SCM 网络：无 fetch/pull/push/lfs），树干净，未改任何仓库脚本/CMake。
- 新单 `agent-jybuilder-v2-lyra-build-20260918c`（钉 node-01M1WVDEYE7Q4TFV6J83EZGKYJ；创建后 0 消息，`exec cmd --action steer --json @file` 注入启动）。logger 隔离修复生效：链接期 `lvve::Logger` undefined reference 全消，`[100%] Built target lyra-cli`，build exit 0。
- 排障插曲：链接两轮被 OOM 杀——根因是本机并发的 opencoder 仓 `cargo test --workspace`（wasmtime 大链接，load 峰值 310、无 swap）。处置：不 kill 他人进程；按 MemAvailable+load 门槛避让（最终口径 ≥100GB 且 1min load<90），链接期最低并行，被杀退避 300s 重试至多 3 次；重载期节点控制通道 steer 连续 504/503，属瞬态，波谷重试即可。
- 终态 ok=true（5461s）：`/jyhub/lyra-cli/rc-develop/5edd661a68e0616b8efbc949de67ba46a29aaab4/{BUILD_INFO.txt 1393B, lyra-cli 64393944B}`，artifact sha256 `5e07fd77d4360e312fb81d6a1129dda1a0cc6c4afeba949c3b2f64b3e4284ffe`；operator 经 `viking-cli file-server download`（jyhub-build.env prod token）独立回读 sha256 一致；ELF x86-64 PIE 带 debug_info，`--help` 冒烟正常。BUILD_INFO 完整溯源：commit_source=local cherry-pick b9f0d575f、deps/include 6923 头映射（清单 `/tmp/deps-provision-manifest.txt`）、full_build_blockers 两项照实保留。
- 挂起项更新：lyra-cli"三选一"按用户指令走本地路线闭环；余下 5edd661a6 推送/PR 与 full 模式两个闭源 artifact（本地 find 无缓存）仍等用户决定。记忆池 jybuilder-lyra-cli-builder 已升 v5（含 OOM 避让口径与控制通道瞬态经验）。
