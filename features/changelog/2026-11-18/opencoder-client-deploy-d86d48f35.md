# opencoder 客户端发布 d86d48f35（node02/node05：49641233f → d86d48f35）

## 发布内容

- 发布目标：node02、node05 的 `/usr/local/bin/opencoder`（节点仅运行客户端二进制，无 systemd 单元）。
- 本版承载 [远端强插走 Interrupt + 退出去挂起](tui-remote-steer-interrupt-quit-unblock.md)（提交 d86d48f35）。
- 产物来源：应用户要求未重新编译，直接采用主工作区现有 release 产物；工作区 `git status` 0 变更，产物版本元数据 `0.1.0 (d86d48f35)` 无 dirty 标记。
- 安装方式：scp 后先校验 SHA256，再按既有惯例生成 `.bak.<时间戳>` 备份，最后原子 `mv` 落位（避免运行中进程 ETXTBSY）。
- 产物 SHA256：`7e0d0052ebae6b91b002c9953b4c04b46ea13be9eccdfaa4bba3e4ba8170642a`。

## 验证

- 两节点 `/usr/local/bin/opencoder --version` → `0.1.0 (d86d48f35)`；SHA256 与源产物一致；`--help` 可用；`/tmp` 无残留。
- 回滚点：node02 `opencoder.bak.20261118213737`、node05 `opencoder.bak.20261118213808`（均为上一版 49641233f）。
- 本机 `/usr/local/bin/opencoder` 不在本次发布范围。

## 测试覆盖

| 项目 | 方式 | 结果 |
|------|------|------|
| 产物元数据 | 源产物 `--version` 无 `-dirty`，与 HEAD d86d48f35 一致 | 通过 |
| 传输完整性 | 节点侧 sha256 与源产物比对 | 通过 |
| 安装后可用性 | 节点侧 `--version`、`--help` | 通过 |

## 补充发布（node05 用户级副本）

- 初次发布后用户实测 `opencoder --version` 仍为 `a6548ed1b-dirty`：node05 用户 `m` 的 PATH 实际命中 `/home/m/.local/bin/opencoder`（另一会话的 dirty 构建，运行中进程 exe 均指向它），遮蔽了 `/usr/local/bin`；node02 无此类副本。
- 已将 `/home/m/.local/bin/opencoder` 同版本替换为 d86d48f35（同目录 `.new` 落位后原子 rename，属主保持 `m:m 755`），用户侧 `--version` → `0.1.0 (d86d48f35)`、`--help` 正常、SHA256 一致。
- 回滚点：`/home/m/.local/bin/opencoder.bak.20261119101036`（a6548ed1b-dirty）。
- 替换时在运行的 3 个旧进程不受影响，持有旧镜像直至重启。
