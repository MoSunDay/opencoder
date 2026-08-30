Commit: (working-tree, nodes smoke 非封闭性修复)

# nodes smoke 非封闭性修复：per-run 注册表隔离 + 节点名唯一化

## 根因（评审定位闭环）

- `scripts/smoke_nodes.sh` 的 server 不带 `--workdir` → store 落入按 cwd 哈希的**共享持久化 DB**（`~/.local/share/opencoder/<digest(cwd)>/`，脚本 cwd 恒为仓库根 → 跨 run 共享）；
- 注册表中滞留的 `smoke-node` 陈旧条目（心跳早已死亡、status 滞留 `idle`）被 checkpoint 1 的「同名 && idle 取 ns[0]」选择器瞬间命中 → ck2 派发绑到死节点 id → 活节点永不认领 → ck3 永久 pending；
- A/B 判别：固定名脚本路径 4 连红 vs 全新名手工等价流程 3s done（2 次）——注册表机制解释全部数据点。

## 修复（`scripts/smoke_nodes.sh`，双保险 + 负载加固）

1. **per-run server workdir + XDG 重定向**：server 挂 `--workdir ${TMP}/srv`（全局参数，须前置 `daemon` 子命令）且 `XDG_DATA_HOME=${TMP}/xdg` → `data_dir_for(workdir)` = `${TMP}/xdg/opencoder/<digest>` → 注册表彻底离开共享 DB，`cleanup` 的 `rm -rf ${TMP}` 连带清退（data root 零泄漏，前后快照 md5 一致）；worker 同样重定向；
2. **节点名 per-run 唯一化**：`smoke-node-$$`，ck1 选择器参数化（`sys.argv[1]`）——即使未来回到共享注册表场景，固定名也不可能再误绑陈旧条目；
3. **轮询预算加固**：readiness/ck1 轮询 30s→90s（16 核 box 上并行 `cargo test --workspace` 编译风暴期间，debug 二进制冷启动可远超 30s）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 双进程冒烟（cargo 注入 debug 二进制 + 看门狗 300s） | `smoke_script_two_process_nodes_flow_passes` | `tests/nodes_smoke_proc.rs` |
| 冒烟 4 检查点（注册 idle / 派发回执 / 终态 / 详情+`?status=`+`?node_id=`过滤+session 反查） | checkpoint 1–4 | `scripts/smoke_nodes.sh` |

- 脚本级验证：`SMOKE NODES PASSED`（4 checkpoints 全✅，load 120 编译风暴下仍稳），且 data root 前后 md5 一致（零泄漏）
- 全量回归：`cargo test --workspace --no-fail-fast` → **3686 passed / 0 failed**（239 个测试二进制，CARGO-EXIT=0，当次实跑；运行树 = b9e14a1 + 工作树 WIP，`smoke_script_two_process_nodes_flow_passes ... ok` 在统一通道内转绿）
