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
