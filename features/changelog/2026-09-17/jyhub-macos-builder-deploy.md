Commit: (working-tree, ops-only)

# jyhub-macos-builder 节点部署与剪映 macOS 构建链路打通（无代码改动）

## 链路拓扑
- VM `172.30.255.2`（aarch64 Ubuntu 24.04，ssh alias `macos-arm-linux-100-86-225-228`，须显式 `-i /root/.ssh/id_ed25519_10_37_35_13_20260811`）运行 opencoder-agent（systemd `opencoder-agent-jyhub`），经本机反向隧道（`jyhub-agent-tunnel.service`，`-R 127.0.0.1:18081`）注册为本机 opencoder-server（127.0.0.1:3039 / 代理 18081）节点 `jyhub-macos-builder`（node-01M2NMM2G7DF1NPVAEBKJ9PYGT，online=True）。
- VM → macos-host（root@192.168.5.2）免密 `/root/.ssh/id_ed25519_jybuild`；mac git 访问 code.byted.org 走 `/var/root/.ssh/git-gw.sh`（已配 `core.sshCommand`），fetch 慢（中转链路分钟级）。
- 凭证：viking-auth principal `cicd-jyhub-build`（token id=1266），VM `/etc/viking/cicd-jyhub-build.env`+`.token`（0600），本机 `/root/.viking/jyhub-build.env`（0600）；file-server prod token 经 `VIKING_AUTH_PROD_ISSUED_TOKEN` 传递（非 VIKING_AUTH_TOKEN）。

## 构建入口与产物
- VM `/opt/opencoder-agent/scripts/jyhub-build.sh`（--commit/--branch/--mode manifest|full）：guard(exit 42 脏工作区)→checkout→增量构建→manifest→上传 file-server `/jyhub/VideoFusion-win/<branch-slug>/<commit>/BUILD_INFO.txt`，成功输出一行 JSON（ok/seconds/manifest）。
- 构建命令 `cmake --build build/release-ninja-mac --target VideoFusion-macOS`（preset release-ninja-mac；产物 ~2.4G `.app`）；**必须指定 --target**：全量构建会编 CEF demo target，其 `ibtool` 需要完整 Xcode（CommandLineTools 下必失败）。
- 优化：checkout 前先 `git cat-file -e <commit>^{commit}`，对象已在本地则跳过 `git fetch`（指定历史 commit 时秒级就绪；新 commit 才走慢速 fetch）。

## 踩坑记录
- file-server `POST /directories` **不支持递归建目录**：父 entry 不存在返回 404 `entry not found`；已存在返回 409 `entry_exists`（幂等可跳过）。fs-mkdir.py 已改为沿祖先链逐级创建；viking-cli upload 不会隐式建父目录。
- `cmake --build ... | tail -5` 管道吞退出码：远端 ssh 脚本须 `set -e -o pipefail`。
- python 工具（fs-mkdir.py/fs-raw.py）token 读取链：VIKING_AUTH_TOKEN → PROD_ISSUED → DEV_ISSUED；注意 heredoc 无引号会先被外层 shell 展开变量。
- branch slug：`tr '/+' '--'` 为逐字符映射，`rc/develop` → `rc-develop`（单横线，非 `rc--develop`）。
- manifest `app_size` 用 `stat -f %z` 对目录只得 inode 大小（96），已改 `du -sk`。

## 平台 REST 派发现状（遗留项）
- `POST /api/sessions`、`POST /api/nodes/:id/tasks` 的 execution preflight 要求 provider key，当前 server 无 `openai` provider 配置（`/api/models` 仅默认 openai/gpt-4o-mini）→ 平台侧派发阻塞，属全局 provider/harness 配置缺口（如 harness=codex+profile 或补 provider key），与 jyhub 节点部署无关。
- 冒烟已用直接触发方式验收：commit `3e4c2d9688c2c327b26742bc25b8146b6545f112`（rc/develop）manifest 模式 6 秒完成，BUILD_INFO.txt 已落 file-server 并下载比对一致。

## 复验与 full 模式验收（2026-09-17 凌晨执行回填）

- **暂存区一致性**：本机 `/tmp/jyhub-deploy/scripts/fs-raw.py` 为带缩进缺陷的旧拷贝（token fallback 补丁遗留，`py_compile` 恰好通过但运行时 token 读取在 `def main()` 外），VM 侧为修复版；已从 VM 同步回本机（0700），双端编译通过，VM 实测 raw GET entries HTTP 200。
- **新缺陷（复验暴露）**：manifest 段改为双引号远端脚本后，`$(du -sk ...)` 与 build 段 `$PATH` 未转义，在 VM 本地求值（路径不存在 → `app_size_kb` 为空）。已补 `\$(...)`/`\$PATH` 转义，同步 VM 后复跑冒烟：`app_size_kb=2493436`（≈2.4G），远端 BUILD_INFO.txt 与本地拉取版逐字节一致。
- **full 模式实跑验收（风险 #1 闭环）**：打包（mac 端 ~2 分钟）→ scp 回 VM（997MB ~2 分钟）→ viking-cli 上传在 complete 阶段报 404 `entry not found`（两次复现，cancel 亦 404/超时），但**数据实际落库**：服务端 entry size 与本地一致，下载回读 sha256 与本地完全一致（a912885f…）。定性为 viking-cli 大文件 complete 应答缺陷（会话提交后被清理，complete 调用扑空）。缓解：jyhub-build.sh 在 upload 失败路径回读远端 entry size 比对本地，一致判成功、不一致 exit 44（仅失败路径触发）。复跑 full 端到端：`{"ok": true, "mode": "full", "seconds": 878}`（含重传共 ~15 分钟）；第二次 overwrite 上传同样经下载回读校验（b4785181… 与当轮本地 sha256 一致，两轮 tar.gz 因 gzip 时间戳字节不同属预期）。
- **清理补全**：远端 `/jyhub/smoke`（递归）、`/jyhub/bw-test-5m.bin` 已删；本机 `/tmp/jyhub-smoke.txt`、暂存区 `__pycache__` 已删；本机暂存区 jyhub-build.sh 0700、.bak 0600。
- **附带条件 ① 完成**：`docs/cross-compile-aarch64.md` 与 `features/changelog/2026-09-17/` 已 commit（055508bd，ops-only 无源码改动）；`.cargo/config.toml` 被 `.gitignore` 忽略（宿主本机专用，有意不入库）。
- **遗留项不变**：平台 REST 派发（`/api/sessions`、`/api/nodes/:id/tasks`）被 execution preflight（缺 `openai` provider key）阻塞，属平台侧配置缺口，与本链路无关。

## 2026-09-17 闭环:REST 派发 API Key preflight + cicd-jyhub-builder 上传规范(无代码改动)

### REST 派发 API Key 闭环
- 根因:节点任务 preflight(`crates/worker/src/operations/create.rs` `prepare()` → `config.resolve_endpoint()` → `crates/core/src/config.rs` `api_key_for()`)要求 provider key;config 从 VM `/opt/opencoder-agent/.opencoder/config.json` 或 `/root/.opencoder/config.json` 加载(`crates/core/src/config/env.rs` `config_candidates`),两处原本均缺。
- 修复:本机 config.json 同步至 VM `/root/.opencoder/config.json`(0600,model `codemaster/glm-5.3-flash`),**必须摘除 `agent.agents_dir` 与 `dag.wasm_dir` 段**——Linux 上 `check_mount`(`crates/worker/src/resources.rs`)强制 agents_dir 为只读 NFS 挂载,VM 无 NFS 会 preflight 失败。
- 验证:operator 冒烟任务 LLM 正常回复;前日遗留项「REST 派发被 preflight 阻塞」就此闭环。

### cicd-jyhub-builder principal 与上传规范
- viking-auth admin API(HMAC 签名请求)创建 service principal `cicd-jyhub-builder`(id 157363,token id=1271,有效期 2027-09-17);凭证 VM `/etc/viking/cicd-jyhub-builder.env`+`.token`、本机 `/root/.viking/jyhub-builder.env`(均 0600)。旧 principal `cicd-jyhub-build`(id 156341)继续负责 `/jyhub/VideoFusion-win/<branch-slug>/<commit>/` 归档。
- `jyhub-build.sh` 新增第 6 段(full 模式):`fs-mkdir.py` 建目录(`POST /directories` 不递归,已存在 409 幂等)→ 上传 builder 个人命名空间 `macos/<branch-slug>/<commit>.app`(branch-slug 由 `tr '/+' '--'` 生成,如 `rc-develop`)→ 下载回读 sha256 校验(不一致 exit 45)→ 最终 JSON 增加 `artifact_remote`/`artifact_sha256`。第 5 段旧路径 full 上传保留(归档冗余,未来可裁剪省 ~15 分钟)。
- SKILL.md 同步双凭证、新产物路径与 exit 45 排障;VM `/opt/opencoder-agent/.opencoder/skills.json` 启用 `jyhub-macos-build`,REST 派发任务中 skill 提醒注入正常。

### 实跑交付验证(execution `operator-jyhub-macos-build-3e4c2d96`)
- 任务:commit `3e4c2d9688c2c327b26742bc25b8146b6545f112`,branch `rc/develop`,mode full;增量构建(ninja no work to do)→ mac 端打包 1.04G tar.gz → scp 回 VM(~6 分钟)→ 双路上传 → 下载回读校验,总耗时 2291 秒,exit 0。
- 脚本输出:`{"ok": true, "mode": "full", "seconds": 2291, "artifact_remote": "macos/rc-develop/3e4c2d9688c2c327b26742bc25b8146b6545f112.app", "artifact_sha256": "ef86919e3f2c98ff8c3a418db1e16a333cc87717d21f245f8fb6809c3e3b431d", ...}`;`/jyhub/.../app.tar.gz` 与 `macos/rc-develop/<commit>.app` 双路 entry size 均为 1045212740。
- **本机独立回读终验**:`cicd-jyhub-builder` 凭证 `viking-cli file-server download macos/rc-develop/<commit>.app`(1.04GB 63 秒),sha256 与 `artifact_sha256` 逐字节一致,`tar -tzf` 解包校验 13282 个条目正常(仅 macOS xattr PAX 头警告,无害)。
- 运维要点补充:① 大文件上传期 SSH 会被流量挤占超时,监控走平台事件流 `GET /api/executions/<id>/events?after=N`(kind 为 tool_start/tool_end/text_delta/reasoning_delta);② 本机/VM 的 harness 会话结束时按 cgroup 回收后台子进程,长任务须 `systemd-run` transient service 托管(VM 实测 setsid/nohup 均逃不掉);③ viking-cli 大文件 upload complete 阶段 404 误报复现,仍以下载回读 sha256 为准,脚本内置兜底(size 回读/exit 44、下载回读/exit 45)两路均触发并判定成功。

## 2026-09-17 附:VM 数据盘 80G→200G 扩容(无代码改动)

### 动机
- 打包在 macOS 上进行,但上传 file-server 由 VM 执行(viking-cli 与 `cicd-jyhub-*` 凭证都在 VM),full 模式的 ~1GB tar.gz 须先 scp 回 VM 暂存(`artifacts/<commit>/app.tar.gz`)且不清理,每轮 full 构建约堆 1GB;VM 根盘 19G 仅剩 9.6G。

### 扩容过程
- VM 是 macos-host(192.168.5.2,物理网 10.3.228.165,APFS 433G 可用)上 colima(lima 2.1.4,vz 后端)的 profile `k3s-arm`;数据盘为 lima 磁盘文件 `/Users/bytedance/.colima/_lima/_disks/colima-k3s-arm/datadisk`(80GiB raw 稀疏),VM 内 vdb1(ext4)bind 挂载 docker/containerd/k3s 等。
- **第一次尝试失败**:在"本机→VM→mac"的 ssh 链上顺序执行 stop→备份→truncate→改配置→start,`colima stop` 关机导致 ssh 断开、脚本中止(备份/扩容均未执行);VM 由 launchd KeepAlive 的 colima daemon 于 12:03 自动恢复(数据无损)。**教训:跨跳板 ssh 的多步主机操作必须写成 nohup 自包含脚本落盘执行。**
- 第二次成功(12:08-12:10):nohup 脚本依次 `colima stop` → `cp -c`(APFS clonefile)备份 `datadisk.bak-80g` → `truncate -s 200G datadisk` → 同步 colima 配置 `disk: 80→200` → `colima start`。
- lima guestagent 在 boot 时自动对数据盘 growpart + resize2fs:vdb1 → 197G,**可用 168G**(原 56G)。VM 侧 agent(enabled)自启、本机 `jyhub-agent-tunnel` Restart=always 自动重连、k3s-agent 自启,节点 `jyhub-macos-builder` 重新 online。
- 构建暂存迁移:`/opt/opencoder-agent/artifacts` → 软链至数据盘 `/mnt/lima-colima-k3s-arm/opencoder-artifacts`(脚本零改动),根盘占用降至 7.8G。
- 验证:平台节点 online=True;VM 上传 `macos/smoke-disk.txt` 下载回读内容一致后删除;`k3s-agent` running。
- mac 侧遗留物:`resize-k3s-arm.log`(过程日志)、`datadisk.bak-80g`(APFS clonefile,增量占用小,稳定后可删)、`datadisk.bak-40g`(9 月 2 日扩容旧备份,可删)。

## 2026-09-17 附:file-server 原生 share 机制打通(heyang.amos 维护入口)

### 命名空间与共享模型(实测)
- viking-file-server 命名空间**按 principal 隔离**(用 `cicd-jyhub-build` 凭证列 `cicd-jyhub-builder` 的 `/macos` 返回 404),每个 principal 根目录下固定挂 `/.shared`(共享区,顶层只读)。
- 原生 share 能力(OpenAPI 实测,`GET /api/v1/viking-file-server/openapi.json`):`POST /shares {path, recipient:{kind:user|service, external_id}, access:read|write}`,"Share an owned path to an exact active user or service Principal";管理路由 `GET /shares`、`GET/DELETE /shares/{id}`、`DELETE /shares/{id}/mount`。`/.shared/<share_id>` 即接收方挂载点,`match=subtree`。
- 已创建 share:`share_id=2ffa6afc-2053-4600-b597-0741736be89f`,`/macos` → `heyang.amos`(user,id=1 bootstrap admin),`access=write`;接收方以**本人 token**(`~/.viking/prod-issued-token.env`)经 `viking-cli file-server list/download/delete` 全程验证:可见 `rc-develop/3e4c2d96....app`(1045212740 字节)、上传/删除测试文件成功(写权限确认)。
- **结论:无需共享服务凭证**。heyang.amos 用自己身份经 `/.shared/2ffa6afc-2053-4600-b597-0741736be89f/...` 即可查看与维护(审计归本人、可独立吊销);VM 的 `cicd-jyhub-builder` token(1271)继续专用于流水线上传。若脚本侧确需 builder 身份 token,可经 admin API 另发独立 token(勿复用 VM 凭证)。
- 注意:删除走 `DELETE /entries/delete` + `If-Match` revision,手工裸调易 409 revision_conflict,`viking-cli file-server delete --yes` 会自动刷新 revision;viking-cli 大文件 upload complete 404 误报与之前一致,以下载回读为准。

### 附2:share 扩展到 caibinfeng(2026-09-17 15:45)
- `POST /shares {path:"/macos", recipient:{kind:"user", external_id:"caibinfeng"}, access:"write"}` → share_id `29282a20-5a3f-42c6-9010-907b48367858`(挂载 `/.shared/29282a20-5a3f-42c6-9010-907b48367858`);caibinfeng(user id=3210,active)已确认存在。
- 实测补充:**viking-cli upload 不会自动创建远端父目录**(新子目录上传在 complete 阶段 404,非大文件误报),上传前须以原始 API 建目录(`POST /directories`,fs-raw.py);已存在目录内上传正常。

### 附3:token 泄露事故响应与轮换(2026-09-17 15:56)
- **事故**:`cicd-jyhub-builder` 的 token 明文曾被输出到 AI 对话上下文中(会话记录不可信)。涉及 token id=1271(prefix `vka_894c93703fb9`,principal id=157363)。
- **处置**:经 viking-auth admin API `POST /admin/tokens/1271/revoke` 吊销(2026-09-17 07:52:31 UTC,含误签的悬空 token 1277/1278 一并吊销);重签 token **id=1279**(label `jyhub-builder-file-server-r4`,prefix `vka_70fb9a66235f`,默认有效期 30 天,2026-10-17 到期——admin create-token 不支持自定义 `expires_at`,到期需重新轮换)。
- **轮换落地**:本机 `/root/.viking/jyhub-builder.env`、VM `/etc/viking/cicd-jyhub-builder.env`+`.token`(均 0600)已用新明文覆写;明文仅经 ssh stdin/重定向落盘,全程未回显。VM 实测 `viking-cli file-server list macos/` 正常。
- **规范(长期)**:任何凭证明文不得出现在文档、对话、提交中;对外引用一律用 token id / principal id(heyang.amos=id 1、caibinfeng=id 3210、cicd-jyhub-build=id 156341、cicd-jyhub-builder=id 157363)。admin create-token 响应中明文位于 `data.plain_text_token`,`data.token` 仅为元数据对象(此前误把后者写入凭证文件,已重建)。
- `jyhub-build.sh` 与 SKILL.md 仅引用 env 文件路径,轮换无需改动。临时工具 `/tmp/vauth-api.py` 已清理。

### 附4:token 1279 延期 10 年(2026-09-17 16:05)
- 发现 viking-auth admin 专用延期端点 `POST /admin/tokens/{token_id}/expires-at`(body 必填 `expires_at`+`reason`),token 在位延期,**明文与凭证文件均不变**;create-token 也支持 `expires_in_days`(整数)——此前 `expires_at` 被拒是因为字段名不对。
- 执行:`POST /admin/tokens/1279/expires-at {expires_at:"2036-09-17T00:00:00Z"}` → token id=1279 到期时间 `2026-10-17` → **`2036-09-17`**(整 10 年),revoked_at 仍为 null;VM 实测 `viking-cli file-server list macos/` 延期后正常。
- 附3 中"30 天有效期需到期轮换"的备注就此作废;后续如需再调,直接走 expires-at 端点,无需重签重部署。

### 附5:澄清 /.shared 下的 UUID 不是 token(2026-09-17 16:15)
- **`/.shared` 下一级目录名 = share_id(UUID 形态,如 `2ffa6afc-2053-4600-b597-0741736be89f`),与 token 无关**:token 一律 `vka_` 前缀且已全部轮换(1271 吊销、在用 1279 已延期 10 年)。share_id 只是挂载点名字,单独持有它无法访问任何数据,必须叠加接收方本人 token,泄露无害。
- 实测:`POST /shares` 传 `label`/`name` 等额外字段会被**静默忽略**,挂载点永远是 `/.shared/<share_id>`,原生 share 模型不支持自定义命名——这就是最终形态,不是发布未完成。
- 使用方式:`/.shared/<share_id>/...` 一一映射 owner 的 `/macos/...`。例:heyang.amos 取构建产物 = `/.shared/2ffa6afc-2053-4600-b597-0741736be89f/rc-develop/<commit>.app`;caibinfeng 入口 = `/.shared/29282a20-5a3f-42c6-9010-907b48367858/rc-develop/...`。
- 运维补充:share 列表会保留已 revoke 的记录(带 `revoked_at`),`GET /shares/{id}` 单查可辨;`DELETE /shares/{id}` 需要 `If-Match: <revision>` 头(与 entries 删除同规),revision 冲突 409 时重新 GET 再删。

### 附6:share 安全模型实测与可用性缺陷确认(2026-09-17 16:35)
- **安全质疑回应(实测,非推测)**:`/.shared/<share_id>` 是否等效凭据?结论:**否**。跨身份实测:① heyang.amos 访问 caibinfeng 的挂载点 → `404 share not found`;② 随机伪造 share_id → 同样 404;③ owner 列自己 `/.shared` → 空;④ owner 走挂载点访问自己 share 出去的内容 → 404。挂载点路径在服务端按"当前 token 的 principal 是否为该 share 的 recipient"解析,拿别人的 share_id 访问不到任何数据,share_id 无凭据价值,不构成越权漏洞。
- **但接收方可用性缺陷属实**(已向平台反馈建议):① 挂载点强制 `/.shared/<share_id>`,传 `label`/`name` 被静默忽略;② `GET /shares` 只返回"我作为 owner 发出的"记录,接收方**查不到**"谁 share 给我、源路径是什么";③ 被多方 share 时挂载点全为无语义 UUID,无法对上来源。**建议平台改为 `/.shared/<owner_id>/...` 层级或提供接收方视角的 share 元数据查询。**
- 缓解已部署:owner 在 `/macos/SHARE_INFO.txt` 放共享说明(owner、映射路径、产物命名、维护人),接收方点进任何挂载点第一眼可辨来源(heyang 侧挂载点实测可见)。
- 运维补充:今天 file-server 上传 complete 阶段响应慢,viking-cli 报"complete: context deadline exceeded"超时——**但服务端实际落盘成功**(重试时 `409 target already exists` 证实),与此前"complete 404 误报"同属一类,结论不变:complete 报错一律回读为准;share 挂载视图对刚写入文件有秒级最终一致性延迟。

### 附7:附6 更正与平台侧修复落地(2026-09-17 17:10)
- **更正附6 第②点**:"接收方查不到 share 元数据"结论有误——`GET /shares?direction=received` 一直存在,接收方可查 owner、源路径等元数据(当时只试了默认 direction=outgoing)。被多方 share 时挂载点无语义的问题仍属实。
- **平台已按建议修复**:viking-file-server 现已把挂载点改为 `/.shared/<owner_external_id>/<share_id>` 两级结构(平台侧 commit `e0a959d`,changelog `features/changelog/2026-09-17/share-owner-layered-mounts.md`),`/.shared` 列出共享方目录;存量单段 `/.shared/<share_id>` 路径保持兼容,现有 jyhub-builder 脚本不受影响。`label`/`name` 被忽略的问题依然存在,但两级结构已让来源可辨,SHARE_INFO.txt 缓解继续保留。
