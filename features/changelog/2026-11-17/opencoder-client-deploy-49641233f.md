# opencoder 客户端发布 49641233f（node02/node05：685ec8b09 → 49641233f）

## 发布内容

- 干净构建：主工作区提交后全净（`git status` 0 变更），`cargo build --release --locked -p opencoder`，产物 `opencoder 0.1.0 (49641233f)`（无 dirty 标记）。
- 本版承载 [TranscriptReset 重建离环](../2026-11-16/tui-transcript-reset-offloop.md) 与
  [压缩重建去在途回显](../2026-11-16/tui-compaction-no-inflight-echo.md) 两项修复，
  闭合 [评审 TODO.1](review-tui-transcript-reset-compaction-echo.md)（685ec8b09 不含该 WIP）。
- 发布目标：node02、node05 的 `/usr/local/bin/opencoder`（节点仅运行客户端二进制，无 systemd 单元）。
- 安装方式：scp 后先校验 SHA256，再按既有惯例生成 `.bak.<时间戳>` 备份，最后原子 `mv` 落位（避免运行中进程 ETXTBSY）。
- 产物 SHA256：`47bb2538efdcb59f3de6849280cb73fba93a65cd213bae83b88ef09bb00d92bf`。

## 验证

- 两节点 `/usr/local/bin/opencoder --version` → `0.1.0 (49641233f)`；SHA256 与源产物一致；`--help` 可用；`/tmp` 无残留。
- 回滚点：node02 `opencoder.bak.20261118045013`、node05 `opencoder.bak.20261118045014`（均为上一版 685ec8b09）。
- 本机 `/usr/local/bin/opencoder` 仍为另一会话的 dirty 构建，不在本次发布范围。

## 测试覆盖

| 项目 | 方式 | 结果 |
|------|------|------|
| 干净构建 | release/--locked，版本元数据无 `-dirty` | 通过 |
| 传输完整性 | 节点侧 sha256 与源产物比对 | 通过 |
| 安装后可用性 | 节点侧 `--version`、`--help` | 通过 |
