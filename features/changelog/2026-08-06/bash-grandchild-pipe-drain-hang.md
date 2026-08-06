Commit: (working-tree, pre-initial-commit)

# bash 工具命令自然完成但遗留后台孙进程时不再永久卡死

## 背景
当模型执行会派生后台孙进程的命令（`cmd &`、dev server、构建 worker 等）时，bash 主进程正常退出，`child.wait()` 在 130s 内返回 `Ok`——**未触发超时**。但被 `&` 后台化的孙进程继承了 stdout/stderr 管道的 write-end 且仍存活，导致 drain 任务阻塞在 `pipe.read()` 永远读不到 EOF。而 bash 工具在 runner 中设的是 `None` deadline（无安全网），**没有任何东西能打断这个等待，最终完全卡死**。

对比：超时路径（>130s）是安全的——`handoff()` 先 `kill(-pgid, SIGKILL)` 杀整组、再用 2s 有界等待 drain。本次修复让自然完成路径补齐这两道防线，与 handoff 行为一致。

## 变更

### session: 自然完成路径加进程组 kill + 有界 drain
- **`crates/session/src/tools/bash.rs`**（`execute` 自然完成分支）：在 `unregister(pid)` 之后、drain await 之前，新增 `kill(-pgid, SIGKILL)` 杀整个进程组，使继承管道 write-end 的孙进程死亡、drain 任务随即读到 EOF；并将原本无界的 `stdout_task.await` / `stderr_task.await` 包入 `tokio::time::timeout(Duration::from_secs(2), …)` 有界等待（镜像 `bg.rs::handoff` 的 2s 上限）。两段均置于 `#[cfg(unix)]` 下，非 unix 路径不受影响。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 后台化孙进程泄漏管道时自然完成不卡死 | bash_returns_when_grandchild_leaks_pipe | crates/session/src/tools/bash.rs |
