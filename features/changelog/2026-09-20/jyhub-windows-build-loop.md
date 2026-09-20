Commit: (working-tree, ops-only)

# windows-operator-node 构建链路首次闭环（V-W1..V-W5 · 零代码改动）

2026-09-18 上线的 windows-operator-node 此前只有探针/brain/agent 单向冒烟、从未跑过构建；本轮以 4 张 pinned 内联单 + operator 直连完成「环境盘点 → 构建冒烟 → 制品上传回读 → 团队接入维护」全链路闭环。全部单显式 node_id，未动 dispatch/调度面，未改 win2 侧 scripts/源码。

## 形态修正（V-W1/V-W2 实测，推翻先前两处推断）
- windows-operator-node 的 agent harness 并非 Windows 二进制：它就是 human-os-02 本机的一个 fleet 节点进程（root、POSIX bash），经 `ssh jyhub-win2`（127.0.0.1:22227 反向隧道，Administrator）驱动 Windows 机 win2。先前「运行 Windows 版 node 二进制」与 probe-1 ENOENT 定性均需按此修正：跨平台单的风险不是节点 OS，而是【单内命令的 shell 语义】——落到该节点的单若直接跑 Linux 命令会在 win2 语义下失败，反之亦然。
- win2 已是配好的构建机（非裸机）：VS2022 BuildTools 17.14（MSVC 14.44）+ Qt 6.2.2 msvc2019_64 + Ninja 1.12.1 + CMake 3.31.6 + git 2.55.0/lfs 3.7.1，`D:\JyHub\r1` 为 v3.1 供给根（build-all.cmd quick/full 分阶段脚本、r1-settings.cmd、Submit-BuildStep.ps1 断线安全作业系统、19GB link-cache、source-lock.json）。VS2019 与 jom 缺失：构建链实际用 ninja，local_package.bat 的 VS2019/jom 路径假设过时，不可直跑，正确入口是 `Submit-BuildStep.ps1 -Name win-<stamp> -Command "D:\JyHub\r1\scripts\build-all.cmd quick <ref>"`。
- 既有检出 `D:\JyHub\r1\s\VideoFusion-win`：detached@0b1835ba（release/11.6.0，source-lock clean=true/lfs_verified=true），2026-09-17/18 已有两轮成功构建产物。

## 执行记录（证据：workspace artifacts/test-agent/2026-09-20/jyhub-windows-build/）
- **V-W1** `agent-vw1-env-20260920a`（钉 windows-operator-node，只读盘点）：OS=Win10 企业版 22H2 19045/32C/64G（与手册"Server 2022"不符，以实测为准）；code.byted.org 302 可达；win2 无 viking-cli 且 viking.bytedance.net DNS 不可解析（架构预期：上传在 operator 侧）。
- **V-W2** `agent-vw2-src-20260920a`（供给布局盘点）：确认节点自身=human-os-02、r1 脚本集语义、v3.1 基线（quick 稳态 5–12min，09-18 实测 7m21s）、Package/ 既有 2.96GB 产物、Submit-BuildStep 用法与轮询判据。
- **V-W3** `agent-vw3-build-20260920a`（构建冒烟）：提交 quick 作业 win-20260920vw3（同 ref 热跑）→ **exit 0，10m24.9s**（configure SKIP key 命中 → link-cache restore → ninja no-work → 打包链全过，sign_process rc=0 走 OFF 路径）；产出 4 件：JianyingPro_11_6_0_19072_local_test.7z（1,204,766,160B）、JYInstaller.exe（873,122,806B）、install_kit 7z（5,608,090B）、pdb.7z（32B 占位，GENERATE_PDB=OFF）。首试提交因 bash→powershell 引号传递失败一次（`-Command` 参数被本地 bash 拆引号），单引号包裹整条远端命令后成功。
- **V-W4** operator 直连 + `agent-vw4-upload-20260920a`（钉 human-os-02，hosted session 承载大文件上传）：
  - 拉回通道实测：scp/sftp 过隧道仅 ~0.3MB/s，`ssh jyhub-win2 "python -c seek/read"` stdout 流式 ~9–15MB/s（30 倍）；operator shell 后台进程（含 setsid+ssh 子进程）跨调用被回收 → 大传输采用【前台分块 seek+append 断点续传】（staging /data00/jyhub-staging/videofusion-win/0b1835ba/，chunk.sh），跨机 sha256 逐字节一致。
  - 上传：viking-cli 单调用 >30s 超 operator 调用上限 → 改由 pinned linux agent 会话承载（与 jy-builder 38min 构建同模式）。目录 `/jyhub/VideoFusion-win/release-11.6.0/0b1835ba06fc2ce54fa1a0524c4532bfc34b031e/`（沿用既有 `<project>/<branch-slug>/<commit>/` 规范，project 大小写从既有目录）。BUILD_INFO.txt 含 sign=OFF、toolchain、builder、fleet 执行 id、逐文件 sha256、stale 产物豁免说明。
  - 回读：`agent-vw4-upload-20260920a`（pinned human-os-02 会话承载）收口全绿——两大制品上传 exit 0（sha256 与源一致，revision a70b75ec…/55fafb95…，overwritten=false），下载回读 sha256 逐字节一致（主 7z 21c132ca…c9ee7555f9、JYInstaller 4224c76b…74cdc68a3），远端目录终态 5 条目（两大制品 + BUILD_INFO.txt + install_kit + pdb.7z）size 全对齐；本轮未触发 complete-404 误报路径。
- **V-W5** 团队接入：采纳方案①（windows 分片由 human-os-02 上 operator 派 pinned 内联单，无卡依赖——三张内联单实证卡解析非必需）；jy-builder team 定义不变（三成员继续覆盖 mac/linux 链路）；`jy-builder-negtest2` 处置=retired-in-place（teams 层无 delete 路由且成员不可清空，保持最小单成员定义 + 文档标记，见 build-team.md）；workspace build-team.md 能力矩阵/运维要点已同步。

## 新增运维要点
- 创建单时的 503 `node disconnected` / `node initial index sync pending` 瞬态：assignment 已保留，同 id 幂等重试即可（20–40s 内恢复），勿换 id。
- operator 侧 shell 的后台进程跨工具调用回收（setsid 亦不保 ssh 子进程，macOS 侧先例同因：cgroup 回收）→ 长任务三条路：前台分块、agent 会话承载、systemd-run transient。
- Submit-BuildStep 提交模板：`ssh jyhub-win2 powershell -NoProfile -ExecutionPolicy Bypass -File D:\JyHub\r1\scripts\Submit-BuildStep.ps1 -Name win-<stamp> -Command '<build-all.cmd quick <ref>>'`（整条 -Command 用单引号防本地 bash 拆引号）；轮询 `type D:\JyHub\r1\logs\<JOB>.json` 判 status=complete 且 exit_code=0，轮询命令须携带变化内容防 doom-loop 守卫。
- win2 时间线证据链：active-job.json（当前/最近作业）+ logs/<job>.json/.out/.err + source-lock.json（locked_at 构建期刷新）三处互证同 ref 热跑。
