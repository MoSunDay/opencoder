Commit: 357916b4f4f9c9b4c05c05d20f8536eaef26d5e0

# stats-sync 目标库路径回归导致 kaboo opencode token 断报

## 现象

- kaboo host-report 自 2026-09-03 起不再含 opencode token bucket
  (会话止于 2026-09-03T01:28:31Z,桶内仅剩 codex/traex 两个来源)。
- `/var/log/opencoder-stats-sync.log` 报
  `/data00/github/opencoder-src/scripts/opencoder-to-opencode-stats.py: No such file`。

## 根因(两处)

1. cron 包装器 `opencoder-stats-sync-cron` 指向 worktree
   `/data00/github/opencoder-src`,该 worktree 2026-09-03 前后被清理,
   同步脚本路径消失,每 30 分钟一次的同步静默失败。
2. commit 137d6594(2026-09-05,命名统一)把脚本目标侧常量
   `OPENCODE_DIR` 从 `~/.local/share/opencode` 误改为
   `~/.local/share/opencoder`(源数据目录,其下并无 `opencode.db`),
   即使脚本恢复运行也无法写入 kaboo 采集的 `~/.local/share/opencode/opencode.db`。

## 修复

- `scripts/opencoder-to-opencode-stats.py`:目标侧恢复为
  `~/.local/share/opencode`(常量 `OPENCODE_DIR` 与相关 docstring),
  源侧 `ENCODER_DIR` 保持 `~/.local/share/opencoder` 不变。
- 运维侧:脚本稳定副本落位 `/usr/local/libexec/opencoder/opencoder-to-opencode-stats.py`,
  cron 包装器改指该副本,不再依赖可被清理的 git worktree 路径。
- 同链路 systemd 单元 `opencode-sync.service`(Rust 版同步器,写 project
  `opencoder-usage`,与 python 主链路重叠):修复 `ReadWritePaths`
  (`/data00` 下非挂载点子路径在本机 systemd 241 命名空间下 ENOENT/exit 226),
  并停用 `opencode-sync.timer` 防止双链路重复上报。

## 验证

- 手动全量增量:changed sessions=902,写入 676 会话 / 44325 条消息,
  水位推进至 2026-09-17T13:39Z;opencode.db `message.max(time_created)`
  恢复到当前时间,global project 会话 6265→6941。
