# opencoder 客户端发布 685ec8b09（node02/node05：8f9fea563 → 685ec8b09）

## 发布内容

- 干净构建：临时 git worktree 固定于 HEAD `685ec8b09`（主工作区含未提交 WIP，不纳入发布），`cargo build --release --locked -p opencoder`，产物 `opencoder 0.1.0 (685ec8b09)`（无 dirty 标记）。
- 发布目标：node02、node05 的 `/usr/local/bin/opencoder`（节点仅运行客户端二进制，无 systemd 单元）。
- 安装方式：scp 后先校验 SHA256，再按既有惯例生成 `.bak.<时间戳>` 备份，最后原子 `mv` 落位（避免运行中进程 ETXTBSY）。
- 产物 SHA256：`362bb4bf850ec38796a1b7554e735e2b4fda82931a396cfeb48e1a0b9dc8f659`。

## 验证

- 两节点 `/usr/local/bin/opencoder --version` → `0.1.0 (685ec8b09)`；SHA256 与源产物一致；`/tmp` 无残留。
- 回滚点：node02 `opencoder.bak.20261117223433`、node05 `opencoder.bak.20261117223422`（均为旧版 8f9fea563）。
- node02 存在发布前已在运行的 opencoder 进程：原子替换不影响其执行，旧进程下次启动自动进入新版本。
- 本机 `/usr/local/bin/opencoder` 为另一会话的 dirty 构建（685ec8b09-dirty），不在本次发布范围。

## 测试覆盖

| 项目 | 方式 | 结果 |
|------|------|------|
| 干净构建 | release/--locked，版本元数据无 `-dirty` | 通过 |
| 传输完整性 | 节点侧 sha256 与源产物比对 | 通过 |
| 安装后可用性 | 节点侧 `--version`、`--help` | 通过 |
