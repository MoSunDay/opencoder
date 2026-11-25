# opencoder 客户端发布 41bb34f14-dirty（node02/node05：d86d48f35 → 41bb34f14-dirty）

## 发布内容

- 发布目标：node02、node05 的 `/usr/local/bin/opencoder`（节点仅运行客户端二进制，无 systemd 单元）；node05 另含遮蔽 `PATH` 的用户级副本 `/home/m/.local/bin/opencoder`。
- 产物来源：应用户要求未重新编译，直接采用本机当前可用产物 `/usr/local/bin/opencoder`（与 `target/release/opencoder` 内容一致、SHA256 相同；2026-11-25 21:05 构建）。
- 版本元数据 `0.1.1 (41bb34f14-dirty)`：基线为 `41bb34f14`（0.1.1 线，含 [DAG clone3 收尾](dag-clone3-pidfd-signal.md)），**带 `-dirty` 标记**——构建时主工作区含未提交改动（`git status` 可见 clone3 沙箱、store 报告链路、CI workflow 等修改），这些改动随本版一并上线，无法从产物反推精确源码状态。
- 安装方式：scp 至同目录 `.new.<时间戳>` → 节点侧 SHA256 校验 → `.new` 上先做 `--version`/`--help` 预检（同时覆盖 glibc 兼容性：本机 2.35，node02 2.35，node05 2.39）→ `cp -p` 生成 `.bak.<时间戳>` 备份 → 原子 `mv` 落位（避免运行中进程 ETXTBSY）。
- 产物 SHA256：`d2b25beeee1a5e36202efb38c3551e56633e8a56848c12814cbb0ebb601049dc`（36551360 字节）。

## 验证

- 两节点 `/usr/local/bin/opencoder --version` → `0.1.1 (41bb34f14-dirty)`；SHA256 与源产物一致；`--help` 可用；`.new` 临时文件无残留。
- node05 用户级副本 `/home/m/.local/bin/opencoder` 同步替换（`m:m 755`，同 `.new` + 原子 rename）；以 `m` 登录 shell 验证 `command -v opencoder` 命中该副本且版本为新版。
- 回滚点：node02 `opencoder.bak.20261125211200`、node05 `opencoder.bak.20261125211200` 及 `/home/m/.local/bin/opencoder.bak.20261125211200`（均为上一版 d86d48f35）。
- node02 发布前已在运行的 TUI 进程（PID 1876334）不受原子替换影响，持有旧镜像直至退出，下次启动进入新版本。

## 测试覆盖

| 项目 | 方式 | 结果 |
|------|------|------|
| 产物元数据 | 源产物 `--version` 读取并如实记录 `-dirty` | 通过（带 dirty 标记，见发布内容） |
| 传输完整性 | 节点侧 sha256 与源产物比对（系统副本 + 用户级副本） | 通过 |
| 安装后可用性 | 节点侧落位前 `.new --version`/`--help` 预检，落位后复核 | 通过 |
| 用户侧生效 | node05 以 `m` 登录 shell 解析 `command -v opencoder` 与版本 | 通过 |

## 后续建议

- 若需可溯源的干净发布，提交当前 WIP 后按 [685ec8b09 发布记录](../2026-11-17/opencoder-client-deploy-685ec8b09.md) 的 worktree 干净构建流程重发。
